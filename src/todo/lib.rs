#[macro_use]
extern crate tracing;

#[cfg(feature = "yaque")]
pub mod yaque_adapter;

#[cfg(test)]
pub mod test;

pub mod open;
pub use open::*;

use std::sync::Arc;
use std::borrow::Cow;

use err_derive::Error;

#[derive(Clone)]
pub struct Todo(pub(crate) Arc<dyn ITodo>);
#[derive(Clone)]
pub struct Queue(pub(crate) Arc<dyn ITodo>, usize);

#[derive(Debug, Error)]
#[error(display = "{}", _0)]
pub struct Error(pub Cow<'static, str>);

pub type Result<T> = std::result::Result<T, Error>;

impl From<std::io::Error> for Error {
	fn from(e: std::io::Error) -> Error {
		Error(format!("IO: {}", e).into())
	}
}

pub(crate) trait ITodo: Send + Sync {
    fn open_queue(&self, name: &str) -> Result<usize>;
    fn submit(&self, queue_id: usize, k: &[u8], v: &[u8]) -> Result<()>;
    fn reserve(&self, queue_id: usize) -> Result<Option<(Vec<u8>, Vec<u8>)>>;
}

impl Todo {
    pub fn open_queue<S: AsRef<str>>(&self, name: S) -> Result<Queue> {
        let queue_id = self.0.open_queue(name.as_ref())?;
        Ok(Queue(self.0.clone(), queue_id))
    }
}

impl Queue {
    pub fn submit(&self, k: &[u8], v: &[u8]) -> Result<()> {
        self.0.submit(self.1, k, v)
    }

    pub fn reserve(&self) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        self.0.reserve(self.1)
    }
}
