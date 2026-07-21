use super::{BackendEvent, BackendKind, BackendLaunch, ProxyBackend};
use anyhow::{Result, anyhow};
use proxy_observe::ObserveRegistry;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

pub struct LeafBackend {
    rt_id: leaf::RuntimeId,
    thread: Option<JoinHandle<()>>,
    tx: Sender<BackendEvent>,
    rx: Receiver<BackendEvent>,
}

impl LeafBackend {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            rt_id: 11,
            thread: None,
            tx,
            rx,
        }
    }

    fn cleanup_finished_thread(&mut self) {
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| thread.is_finished())
        {
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

impl ProxyBackend for LeafBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Leaf
    }

    fn start(&mut self, launch: BackendLaunch) -> Result<()> {
        self.cleanup_finished_thread();
        if leaf::is_running(self.rt_id) {
            return Err(anyhow!("leaf is already running"));
        }

        let config = launch.config_path.to_string_lossy().to_string();
        leaf::test_config(&config)?;

        let tx = self.tx.clone();
        let rt_id = self.rt_id;
        self.thread = Some(thread::spawn(move || {
            let _ = tx.send(BackendEvent::Info("leaf starting".to_string()));
            let result = leaf::util::start_with_lifecycle(
                rt_id,
                leaf::StartOptions {
                    config: leaf::Config::File(config),
                    auto_reload: launch.auto_reload,
                    runtime_opt: leaf::RuntimeOption::MultiThreadAuto(default_stack_size()),
                },
            );
            match result {
                Ok(()) => {
                    let _ = tx.send(BackendEvent::Stopped { error: None });
                }
                Err(e) => {
                    let message = e.to_string();
                    let _ = tx.send(BackendEvent::Error(message.clone()));
                    let _ = tx.send(BackendEvent::Stopped {
                        error: Some(message),
                    });
                }
            }
        }));

        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        if leaf::is_running(self.rt_id) {
            if !leaf::shutdown(self.rt_id) {
                return Err(anyhow!("failed to send leaf shutdown signal"));
            }
        }
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        leaf::reload(self.rt_id)?;
        Ok(())
    }

    fn is_running(&self) -> bool {
        leaf::is_running(self.rt_id)
    }

    fn observe_registry(&self) -> Option<ObserveRegistry> {
        leaf::runtime_manager(self.rt_id).map(|runtime| runtime.observe_registry())
    }

    fn drain_events(&mut self) -> Vec<BackendEvent> {
        self.cleanup_finished_thread();
        self.rx.try_iter().collect()
    }
}

#[cfg(debug_assertions)]
fn default_stack_size() -> usize {
    2 * 1024 * 1024
}

#[cfg(not(debug_assertions))]
fn default_stack_size() -> usize {
    256 * 1024
}
