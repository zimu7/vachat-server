use std::path::Path;

use parking_lot::Mutex;
use sled::Db;

use crate::{sequence::Sequence, Messages, Result};

const MSG_SEQUENCE: u8 = 1;

pub struct MsgDb {
    pub(crate) db: Db,
    msg_sequence: Mutex<Sequence>,
}

impl Drop for MsgDb {
    fn drop(&mut self) {
        self.msg_sequence.lock().release(&self.db);
    }
}

impl MsgDb {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = sled::open(path)?;
        let msg_sequence = Mutex::new(Sequence::new(&db, MSG_SEQUENCE)?);
        Ok(Self { db, msg_sequence })
    }

    #[inline]
    pub fn messages(&self) -> Messages<'_> {
        Messages { db: self }
    }

    pub(crate) fn generate_msg_id(&self) -> Result<i64> {
        self.msg_sequence.lock().generate_id(&self.db)
    }

    /// Get the maximum message ID in the database
    pub fn get_max_msg_id(&self) -> Result<Option<i64>> {
        // sled keys are sorted by byte order, MSG/ prefix + big-endian i64
        // so the last key starting with MSG/ is the maximum message ID
        for item in self.db.iter().rev() {
            let (key, _) = item?;
            if key.starts_with(b"MSG/") && key.len() == 12 {
                let mid = i64::from_be_bytes(key[4..12].try_into().unwrap());
                return Ok(Some(mid));
            }
        }
        Ok(None)
    }
}
