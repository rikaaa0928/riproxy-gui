use anyhow::Result;
use proxy_observe::ObserveRegistry;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

#[cfg(feature = "backend-leaf")]
mod leaf_backend;
#[cfg(feature = "backend-rog")]
mod rog_backend;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    Leaf,
    Rog,
}

impl BackendKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Leaf => "Leaf",
            Self::Rog => "Rog",
        }
    }

    pub fn is_available(self) -> bool {
        match self {
            Self::Leaf => cfg!(feature = "backend-leaf"),
            Self::Rog => cfg!(feature = "backend-rog"),
        }
    }
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Clone, Debug)]
pub struct BackendLaunch {
    pub config_path: PathBuf,
    pub auto_reload: bool,
}

#[derive(Clone, Debug)]
pub enum BackendEvent {
    Info(String),
    Error(String),
    Stopped { error: Option<String> },
}

pub trait ProxyBackend: Send {
    fn kind(&self) -> BackendKind;
    fn start(&mut self, launch: BackendLaunch) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
    fn reload(&mut self) -> Result<()>;
    fn is_running(&self) -> bool;
    fn observe_registry(&self) -> Option<ObserveRegistry>;
    fn drain_events(&mut self) -> Vec<BackendEvent>;
}

pub fn available_kinds() -> Vec<BackendKind> {
    [BackendKind::Leaf, BackendKind::Rog]
        .into_iter()
        .filter(|kind| kind.is_available())
        .collect()
}

pub fn create_backend(kind: BackendKind) -> Result<Box<dyn ProxyBackend>> {
    match kind {
        BackendKind::Leaf => {
            #[cfg(feature = "backend-leaf")]
            {
                Ok(Box::new(leaf_backend::LeafBackend::new()))
            }
            #[cfg(not(feature = "backend-leaf"))]
            {
                Err(anyhow::anyhow!("leaf backend is not enabled"))
            }
        }
        BackendKind::Rog => {
            #[cfg(feature = "backend-rog")]
            {
                Ok(Box::new(rog_backend::RogBackend::new()))
            }
            #[cfg(not(feature = "backend-rog"))]
            {
                Err(anyhow::anyhow!("rog backend is not enabled"))
            }
        }
    }
}
