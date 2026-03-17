//! Single process: pod gateway + voice pipeline. Pod audio → STT → wake/LLM → TTS → pod.
//! Run this instead of (pod-gateway + desktop-runner) when using the pod as the only mic/speaker.

use core_config::Config;
use core_llm::OllamaLlmStream;
use core_observability::{
    init_json_logging, record_memory_load, record_memory_load_duration, register_metrics,
};
use core_skills::{
    HueSmartHomeSkill, MacOsMusicSkill, OpenMeteoDistanceSkill, OpenMeteoTimeSkill,
    OpenMeteoWeatherSkill, SqliteMemorySkill,
};
use core_stt::WhisperSttStream;
use core_tts::PiperTtsSink;
use desktop_runner::{
    build_effective_system_prompt, install_ctrlc_shutdown_handler_with_cleanup,
    resolve_startup_location, ContinuousRunOptions, DesktopRuntime, LlmIntentClassifier,
    MemoryStore, PodIngestCapture, RoutingTtsSink, SkillRunContext,
};
use pod_gateway::run_gateway;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use tokio::sync::mpsc as tokio_mpsc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = init_json_logging();
    register_metrics();
    let config = Config::load(Path::new("config.json"))?;

    let addr: SocketAddr = config
        .pod_bind
        .parse()
        .unwrap_or_else(|_| "0.0.0.0:8765".parse().unwrap());
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "pod gateway listening");

    let (gateway_tx, mut gateway_rx) = tokio_mpsc::unbounded_channel();
    let (sync_tx, sync_rx) = mpsc::sync_channel::<pod_gateway::PodIngestEvent>(256);
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("bridge runtime");
        while let Some(event) = rt.block_on(async { gateway_rx.recv().await }) {
            let _ = sync_tx.send(event);
        }
    });

    let (egress_tx, egress_rx) = tokio_mpsc::unbounded_channel();
    let (tap_tx, mut tap_rx) = tokio_mpsc::unbounded_channel::<()>();
    let (cancel_tx, cancel_rx) = tokio::sync::broadcast::channel(1);
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::unbounded_channel();
    let cancel_tx_tap = cancel_tx.clone();
    tokio::spawn(async move {
        while tap_rx.recv().await.is_some() {
            let _ = cancel_tx_tap.send(());
        }
    });
    tokio::spawn(async move {
        let _ = run_gateway(listener, gateway_tx, egress_rx, Some(tap_tx)).await;
    });

    let mut capture = PodIngestCapture::new(sync_rx);
    let mut stt = WhisperSttStream::new(Path::new(&config.stt.whisper_model_path))?;

    let weather_skill = OpenMeteoWeatherSkill::new();
    let time_skill = OpenMeteoTimeSkill::new();
    let distance_skill = OpenMeteoDistanceSkill::new();
    let smart_home_skill = if config.smart_home.hue.enabled {
        match (
            config.smart_home.hue.bridge_host.as_deref(),
            config.smart_home.hue.app_key.as_deref(),
        ) {
            (Some(host), Some(key)) => Some(HueSmartHomeSkill::new(
                host,
                key,
                &config.smart_home.hue.default_light_name,
            )),
            _ => None,
        }
    } else {
        None
    };
    let media_skill = if config.media.macos_music.enabled {
        Some(MacOsMusicSkill::new())
    } else {
        None
    };
    let media_shutdown = media_skill.clone();
    let cleanup = Arc::new(move || {
        if let Some(skill) = media_shutdown.as_ref() {
            let _ = skill.shutdown();
        }
    });
    install_ctrlc_shutdown_handler_with_cleanup(shutdown_tx, Some(cleanup))?;
    let memory_skill = if config.memory.enabled {
        SqliteMemorySkill::new(Path::new(&config.memory.sqlite_path)).ok()
    } else {
        None
    };
    let resolved_location = resolve_startup_location(&config, &weather_skill).await;

    let (memory_store, llm_system_prompt) = if config.memory.enabled {
        let path = Path::new(&config.memory.path);
        let t0 = Instant::now();
        let store = MemoryStore::load(path, &config.memory);
        record_memory_load_duration(t0.elapsed());
        record_memory_load();
        let prompt = build_effective_system_prompt(
            &config,
            config.llm.system_prompt.as_deref(),
            resolved_location.as_ref(),
            Some(&store),
        );
        (Some(Arc::new(tokio::sync::Mutex::new(store))), prompt)
    } else {
        (
            None,
            build_effective_system_prompt(
                &config,
                config.llm.system_prompt.as_deref(),
                resolved_location.as_ref(),
                None,
            ),
        )
    };

    let llm = OllamaLlmStream::new(
        config.ollama_url.clone(),
        config.model.clone(),
        config.llm.short_replies,
        config.llm.max_output_tokens,
        llm_system_prompt,
    );
    let intent_classifier = LlmIntentClassifier::new(&llm);
    let piper = PiperTtsSink::new(Path::new(&config.tts.piper_model_path))?;
    let mut tts = RoutingTtsSink::new(piper, Some(egress_tx));

    let mut runtime = DesktopRuntime::new(config.clone());

    info!("pod-voice running: say wake word + question on the pod");
    let run_future = runtime.run_continuous(
        &mut capture,
        &mut stt,
        &llm,
        &mut tts,
        ContinuousRunOptions::<core_search::HttpSearchProvider> {
            search: None,
            cancel_rx,
            max_turns: None,
            skills: SkillRunContext {
                intent_classifier: Some(&intent_classifier),
                weather_skill: Some(&weather_skill),
                time_skill: Some(&time_skill),
                distance_skill: Some(&distance_skill),
                smart_home_skill: smart_home_skill.as_ref().map(|s| s as _),
                assistant_skill: None,
                media_skill: media_skill.as_ref().map(|s| s as _),
                memory_skill: memory_skill.as_ref().map(|s| s as _),
                computer_skill: None,
                resolved_location: resolved_location.as_ref(),
                memory: memory_store.clone(),
                policy: None,
            },
        },
    );

    let mut ctrlc_requested_exit = false;
    let run_result: Result<(), Box<dyn std::error::Error + Send + Sync>> = tokio::select! {
        res = run_future => {
            match res {
                Ok(stats) => {
                    info!(?stats, "pod-voice stopped");
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("ctrl-c received, shutting down pod-voice");
            let _ = cancel_tx.send(());
            ctrlc_requested_exit = true;
            Ok(())
        }
        _ = shutdown_rx.recv() => {
            info!("ctrl-c handler requested shutdown for pod-voice");
            let _ = cancel_tx.send(());
            ctrlc_requested_exit = true;
            Ok(())
        }
    };
    if let Some(skill) = media_skill.as_ref() {
        let _ = skill.shutdown();
    }
    if ctrlc_requested_exit {
        std::process::exit(0);
    }
    run_result
}
