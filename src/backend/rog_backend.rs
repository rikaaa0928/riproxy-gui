use super::{BackendEvent, BackendKind, BackendLaunch, ProxyBackend};
use anyhow::{Result, anyhow};
use proxy_observe::ObserveRegistry;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use tokio_util::sync::CancellationToken;

pub struct RogBackend {
    thread: Option<JoinHandle<()>>,
    shutdown: Option<CancellationToken>,
    observe_registry: Option<ObserveRegistry>,
    tx: Sender<BackendEvent>,
    rx: Receiver<BackendEvent>,
}

impl RogBackend {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            thread: None,
            shutdown: None,
            observe_registry: None,
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
            self.shutdown = None;
        }
    }
}

impl ProxyBackend for RogBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Rog
    }

    fn start(&mut self, launch: BackendLaunch) -> Result<()> {
        self.cleanup_finished_thread();
        if self.is_running() {
            return Err(anyhow!("rog is already running"));
        }

        let config = rog::load_config_file(&launch.config_path)?;
        let observe_registry = ObserveRegistry::new();
        let thread_registry = observe_registry.clone();
        let shutdown = CancellationToken::new();
        let thread_shutdown = shutdown.clone();
        let tx = self.tx.clone();

        self.observe_registry = Some(observe_registry);
        self.shutdown = Some(shutdown);
        self.thread = Some(thread::spawn(move || {
            let _ = tx.send(BackendEvent::Info("rog starting".to_string()));
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    let message = e.to_string();
                    let _ = tx.send(BackendEvent::Error(message.clone()));
                    let _ = tx.send(BackendEvent::Stopped {
                        error: Some(message),
                    });
                    return;
                }
            };

            let result = runtime.block_on(rog::run(rog::RunOptions {
                config,
                observe_registry: thread_registry,
                shutdown: thread_shutdown,
            }));

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
        if let Some(shutdown) = &self.shutdown {
            shutdown.cancel();
        }
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        Err(anyhow!(
            "rog reload is not available yet; stop and start the profile"
        ))
    }

    fn is_running(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    }

    fn observe_registry(&self) -> Option<ObserveRegistry> {
        self.observe_registry.clone()
    }

    fn drain_events(&mut self) -> Vec<BackendEvent> {
        self.cleanup_finished_thread();
        self.rx.try_iter().collect()
    }
}
