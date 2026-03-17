use core_observability::record_shutdown_signal;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

static CTRL_C_COUNT: AtomicU8 = AtomicU8::new(0);
const CTRL_C_FORCE_EXIT_CODE: i32 = 130;
type CleanupFn = Arc<dyn Fn() + Send + Sync + 'static>;

fn next_ctrlc_action(prev_count: u8) -> CtrlCAction {
    if prev_count == 0 {
        CtrlCAction::RequestShutdown
    } else {
        CtrlCAction::ForceExit
    }
}

enum CtrlCAction {
    RequestShutdown,
    ForceExit,
}

pub fn install_ctrlc_shutdown_handler(
    shutdown_tx: mpsc::UnboundedSender<()>,
) -> Result<(), ctrlc::Error> {
    install_ctrlc_shutdown_handler_with_cleanup(shutdown_tx, None)
}

pub fn install_ctrlc_shutdown_handler_with_cleanup(
    shutdown_tx: mpsc::UnboundedSender<()>,
    cleanup: Option<CleanupFn>,
) -> Result<(), ctrlc::Error> {
    let cleanup = cleanup.unwrap_or_else(|| Arc::new(|| {}));
    ctrlc::set_handler(move || {
        let prev = CTRL_C_COUNT.fetch_add(1, Ordering::SeqCst);
        handle_ctrlc_action(prev, &shutdown_tx, cleanup.as_ref(), &process_exit);
    })
}

fn process_exit(code: i32) {
    std::process::exit(code);
}

fn handle_ctrlc_action(
    prev_count: u8,
    shutdown_tx: &mpsc::UnboundedSender<()>,
    cleanup: &dyn Fn(),
    exit_fn: &dyn Fn(i32),
) {
    let action = next_ctrlc_action(prev_count);
    cleanup();
    match action {
        CtrlCAction::RequestShutdown => {
            record_shutdown_signal("ctrl_c", "request_shutdown");
            let _ = shutdown_tx.send(());
        }
        CtrlCAction::ForceExit => {
            record_shutdown_signal("ctrl_c", "force_exit");
            exit_fn(CTRL_C_FORCE_EXIT_CODE);
        }
    }
}

#[cfg(test)]
fn run_ctrlc_action_for_test(
    prev_count: u8,
    shutdown_tx: &mpsc::UnboundedSender<()>,
    cleanup: &dyn Fn(),
    exit_fn: &dyn Fn(i32),
) {
    handle_ctrlc_action(prev_count, shutdown_tx, cleanup, exit_fn);
}

#[cfg(test)]
mod tests {
    use super::{next_ctrlc_action, run_ctrlc_action_for_test, CtrlCAction};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::sync::Mutex;
    use tokio::sync::mpsc;
    use tokio::sync::mpsc::error::TryRecvError;

    #[test]
    fn first_ctrlc_requests_shutdown() {
        assert!(matches!(next_ctrlc_action(0), CtrlCAction::RequestShutdown));
    }

    #[test]
    fn second_ctrlc_forces_exit() {
        assert!(matches!(next_ctrlc_action(1), CtrlCAction::ForceExit));
        assert!(matches!(next_ctrlc_action(2), CtrlCAction::ForceExit));
    }

    #[test]
    fn first_ctrlc_runs_cleanup_then_sends_shutdown() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let cleanup_calls = Arc::new(AtomicUsize::new(0));
        let cleanup_calls_clone = Arc::clone(&cleanup_calls);
        let cleanup = move || {
            cleanup_calls_clone.fetch_add(1, Ordering::SeqCst);
        };
        let exit_code = Arc::new(Mutex::new(None));
        let exit_code_clone = Arc::clone(&exit_code);
        let exit = move |code: i32| {
            *exit_code_clone.lock().expect("exit lock") = Some(code);
        };

        run_ctrlc_action_for_test(0, &tx, &cleanup, &exit);

        assert_eq!(cleanup_calls.load(Ordering::SeqCst), 1);
        assert_eq!(rx.try_recv(), Ok(()));
        assert_eq!(*exit_code.lock().expect("exit lock"), None);
    }

    #[test]
    fn second_ctrlc_runs_cleanup_then_forces_exit() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let cleanup_calls = Arc::new(AtomicUsize::new(0));
        let cleanup_calls_clone = Arc::clone(&cleanup_calls);
        let cleanup = move || {
            cleanup_calls_clone.fetch_add(1, Ordering::SeqCst);
        };
        let exit_code = Arc::new(Mutex::new(None));
        let exit_code_clone = Arc::clone(&exit_code);
        let exit = move |code: i32| {
            *exit_code_clone.lock().expect("exit lock") = Some(code);
        };

        run_ctrlc_action_for_test(1, &tx, &cleanup, &exit);

        assert_eq!(cleanup_calls.load(Ordering::SeqCst), 1);
        assert_eq!(*exit_code.lock().expect("exit lock"), Some(130));
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
    }
}
