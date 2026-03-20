//! Desktop runner: initialize config and run continuous real audio loop.

use core_audio::CpalCapture;
use core_config::Config;
use core_llm::OllamaLlmStream;
use core_observability::{
    init_json_logging, init_prometheus_exporter, register_metrics, ExporterInitState,
};
use core_observability::{record_memory_load, record_memory_load_duration};
use core_search::HttpSearchProvider;
use core_skills::{
    HueSmartHomeSkill, MacOsAppSwitcherSkill, MacOsClockTimerSkill, MacOsComputerSkill,
    MacOsMessagesSkill, MacOsMusicSkill, MacOsNotesShoppingListSkill, MacOsReminderSkill,
    MacOsVolumeSkill, OpenMeteoDistanceSkill, OpenMeteoTimeSkill, OpenMeteoWeatherSkill,
    SqliteMemorySkill,
};
use core_stt::WhisperSttStream;
use core_tts::PiperTtsSink;
use desktop_runner::{
    build_effective_system_prompt, install_ctrlc_shutdown_handler_with_cleanup,
    resolve_startup_location, ContinuousRunOptions, DesktopRuntime, LlmIntentClassifier,
    MemoryStore, SkillRunContext,
};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

fn init_runtime_observability(config: &Config) -> Result<ExporterInitState, String> {
    register_metrics();
    if !config.service.metrics_enabled {
        return Ok(ExporterInitState::AlreadyRunning);
    }
    init_prometheus_exporter(&config.service.metrics_bind).map_err(|error| error.to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = init_json_logging();
    let config = Config::load(Path::new("config.json"))?;
    match init_runtime_observability(&config) {
        Ok(ExporterInitState::Started) => {
            info!(bind = %config.service.metrics_bind, "prometheus metrics exporter started");
        }
        Ok(ExporterInitState::AlreadyRunning) => {}
        Err(error) => {
            warn!(
                bind = %config.service.metrics_bind,
                %error,
                "failed to initialize prometheus metrics exporter"
            );
        }
    }

    let mut runtime = DesktopRuntime::new(config.clone());
    let mut capture = CpalCapture::from_preferred_name(config.audio.input_device.as_deref())?;
    info!(device = %capture.device_name(), "microphone capture initialized");
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
    let mut tts = PiperTtsSink::new(Path::new(&config.tts.piper_model_path))?;

    let intent_classifier = LlmIntentClassifier::new(&llm);
    let reminder_skill = MacOsReminderSkill::new();
    let computer_skill = MacOsComputerSkill::new();
    let app_switcher_skill = MacOsAppSwitcherSkill::new();
    let message_skill = MacOsMessagesSkill::new();
    let timer_skill = MacOsClockTimerSkill::new();
    let shopping_list_skill = MacOsNotesShoppingListSkill::new();
    let volume_skill = MacOsVolumeSkill::new();

    let (cancel_tx, cancel_rx) = tokio::sync::broadcast::channel(1);
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::unbounded_channel();
    let media_shutdown = media_skill.clone();
    let cleanup = Arc::new(move || {
        if let Some(skill) = media_shutdown.as_ref() {
            let _ = skill.shutdown();
        }
    });
    install_ctrlc_shutdown_handler_with_cleanup(shutdown_tx, Some(cleanup))?;

    let search: Option<HttpSearchProvider> = if config.search_provider.url.is_empty() {
        None
    } else {
        HttpSearchProvider::from_options(
            &config.search_provider.url,
            config.search_provider.api_key.as_deref(),
            config.search_provider.timeout_secs,
        )
        .ok()
    };

    let run_future = runtime.run_continuous(
        &mut capture,
        &mut stt,
        &llm,
        &mut tts,
        ContinuousRunOptions {
            search: search.as_ref(),
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
                computer_skill: Some(&computer_skill),
                app_switcher_skill: Some(&app_switcher_skill),
                reminder_skill: Some(&reminder_skill),
                message_skill: Some(&message_skill),
                timer_skill: Some(&timer_skill),
                shopping_list_skill: Some(&shopping_list_skill),
                volume_skill: Some(&volume_skill),
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
                    info!(?stats, "runtime stopped");
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("ctrl-c received, shutting down");
            let _ = cancel_tx.send(());
            ctrlc_requested_exit = true;
            Ok(())
        }
        _ = shutdown_rx.recv() => {
            info!("ctrl-c handler requested shutdown");
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

#[cfg(test)]
mod tests {
    use core_config::Config;
    use std::net::TcpListener;

    fn reserve_local_bind() -> String {
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(value) => value,
            Err(error) => panic!("failed to reserve local bind: {error}"),
        };
        let addr = match listener.local_addr() {
            Ok(value) => value,
            Err(error) => panic!("failed to read local bind: {error}"),
        };
        drop(listener);
        addr.to_string()
    }

    #[test]
    fn observability_setup_is_idempotent_when_metrics_enabled() {
        let mut config = Config::default();
        config.service.metrics_enabled = true;
        config.service.metrics_bind = reserve_local_bind();
        let first = super::init_runtime_observability(&config);
        assert!(first.is_ok());
        let second = super::init_runtime_observability(&config);
        assert!(second.is_ok());
    }
}
