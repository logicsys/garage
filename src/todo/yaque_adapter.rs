use crate::*;

use std::path::PathBuf;
use std::convert::TryInto;
use std::sync::RwLock;

use yaque::{channel, Receiver, Sender, TrySendError, TryRecvError};

pub struct Yaque {
    base_path: PathBuf,
    // the outer lock prevents opening 2 queues at once
    // the inner locks allow reserving on multiple queues at once
    queues: RwLock<Vec<RwLock<(Sender, Receiver)>>>,
}

impl From<TrySendError<Vec<u8>>> for Error {
	fn from(e: TrySendError<Vec<u8>>) -> Error {
		Error(format!("Yaque: {}", e).into())
	}
}

impl From<TryRecvError> for Error {
	fn from(e: TryRecvError) -> Error {
        let message = match e {
            TryRecvError::Io(ee) => ee.to_string(),
            TryRecvError::QueueEmpty => "empty queue".into(),
        };
		Error(format!("Yaque: {}", message).into())
	}
}

impl Yaque {
    pub fn new(base_path: &PathBuf) -> Self {
        Self {
            base_path: base_path.clone(),
            queues: RwLock::new(vec!()),
        }
    }

    pub fn init(base_path: &PathBuf) -> Todo {
        Todo(Arc::new(Self::new(base_path)))
    }
}

impl ITodo for Yaque {
    fn open_queue(&self, name: &str) -> Result<usize> {
        let mut path = self.base_path.clone();
        path.push(name);
        let mut queues = self.queues.write().unwrap();
        queues.push(RwLock::new(channel(path)?));
        Ok(queues.len() - 1)
    }

    fn submit(&self, queue_id: usize, k: &[u8], v: &[u8]) -> Result<()> {
        let queues = self.queues.read().unwrap();
        let mut queue = queues[queue_id].write().unwrap();
        let job = pack_kv(k, v);
        queue.0.try_send(job)?;
        Ok(())
    }

    fn reserve(&self, queue_id: usize) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        let queues = self.queues.read().unwrap();
        let mut queue = queues[queue_id].write().unwrap();
        let job = queue.1.try_recv();
        match job {
            Ok(j) => Ok(Some(unpack_kv(&j))),
            Err(TryRecvError::Io(e)) => Err(e.into()),
            Err(TryRecvError::QueueEmpty) => Ok(None),
        }
    }
}

fn pack_kv(k: &[u8], v: &[u8]) -> Vec<u8> {
    [&k.len().to_ne_bytes(), k, v].concat().to_vec()
}

fn unpack_kv(job: &[u8]) -> (Vec<u8>, Vec<u8>) {
    const LEN_SIZE: usize = std::mem::size_of::<usize>();
    let key_length_bytes = (&job[..LEN_SIZE]).try_into().unwrap();
    let key_length = usize::from_ne_bytes(key_length_bytes);
    let k = job[LEN_SIZE..LEN_SIZE+key_length].to_vec();
    let v = job[LEN_SIZE+key_length..].to_vec();
    (k, v)
}
