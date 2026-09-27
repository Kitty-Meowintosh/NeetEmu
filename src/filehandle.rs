//! Open file handles, ported from `simulation/FS/FileHeader.java`.

use std::path::{Path, PathBuf};

/// `simulation/FS/OpeningMode.java`, plus `append`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mode {
    pub can_read: bool,
    pub can_write: bool,
    pub truncate: bool,
    pub binary: bool,
    pub create: bool,
    /// Positions the cursor at EOF.
    pub append: bool,
    pub invalid: bool,
}

impl Mode {
    const INVALID: Mode = Mode {
        can_read: false,
        can_write: false,
        truncate: false,
        binary: false,
        create: false,
        append: false,
        invalid: true,
    };

    /// `FileHelper.getMode`, with the `length() > 2` guard lifted.
    pub fn parse(mode: &str) -> Mode {
        let m = mode.trim();
        if m.is_empty() {
            return Mode::INVALID;
        }
        // read, write, truncate, binary, create, append
        let (r, w, t, b, c, a) = match m.to_ascii_lowercase().as_str() {
            "r" => (true, false, false, false, false, false),
            "w" => (false, true, true, false, true, false),
            "a" => (false, true, false, false, true, true),
            "r+" => (true, true, false, false, false, false),
            "w+" => (true, true, true, false, true, false),
            "a+" => (true, true, false, false, true, true),
            "rb" => (true, false, false, true, false, false),
            "wb" => (false, true, true, true, true, false),
            "ab" => (false, true, false, true, true, true),
            "rb+" | "r+b" => (true, true, false, true, false, false),
            "wb+" | "w+b" => (true, true, true, true, true, false),
            "ab+" | "a+b" => (true, true, false, true, true, true),
            _ => return Mode::INVALID,
        };
        Mode {
            can_read: r,
            can_write: w,
            truncate: t,
            binary: b,
            create: c,
            append: a,
            invalid: false,
        }
    }
}

pub struct FileHandle {
    path: PathBuf,
    mode: Mode,
    open: bool,
    cursor: usize,
    buffer: Vec<u8>,
}

type Result<T> = std::result::Result<T, String>;

impl FileHandle {
    /// Loads the whole file up front.
    pub fn open(path: &Path, mode: Mode) -> Result<FileHandle> {
        let mut buffer = Vec::new();
        if !mode.truncate && path.exists() {
            buffer = std::fs::read(path).map_err(|e| e.to_string())?;
        }
        if mode.create && !path.exists() {
            std::fs::write(path, b"").map_err(|e| e.to_string())?;
        }
        let cursor = if mode.append { buffer.len() } else { 0 };
        Ok(FileHandle {
            path: path.to_path_buf(),
            mode,
            open: true,
            cursor,
            buffer,
        })
    }

    fn check_open(&self) -> Result<()> {
        if self.open {
            Ok(())
        } else {
            Err("Attempt to use a closed file".into())
        }
    }

    pub fn flush(&self) -> Result<()> {
        self.check_open()?;
        if self.mode.can_write {
            std::fs::write(&self.path, &self.buffer).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn close(&mut self) -> Result<()> {
        self.flush()?;
        self.open = false;
        self.buffer = Vec::new();
        Ok(())
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// `seek()`, `seek(whence)` and `seek(whence, offset)`.
    pub fn seek(&mut self, whence: Option<&str>, offset: i64) -> Result<i64> {
        let len = self.buffer.len() as i64;
        let Some(whence) = whence else {
            return Ok(self.cursor as i64);
        };
        let target = match whence {
            "set" => offset,
            "cur" => self.cursor as i64 + offset,
            "end" => {
                if offset > 0 {
                    return Err("Invalid offset".into());
                }
                len + offset
            }
            _ => return Err("Invalid option".into()),
        };
        if target < 0 || target > len {
            return Err("Invalid offset".into());
        }
        self.cursor = target as usize;
        Ok(target)
    }

    /// `read("l")`, `read("L")` and `read("a")`, bytes untouched.
    pub fn read_format(&mut self, format: &str) -> Result<Option<Vec<u8>>> {
        self.check_open()?;
        if !self.mode.can_read {
            return Err("Access denied".into());
        }
        if format.chars().count() != 1 {
            return Err("Invalid format".into());
        }
        if self.cursor == self.buffer.len() {
            return Ok(None);
        }
        let rest = &self.buffer[self.cursor..];
        match format {
            "a" => {
                let out = rest.to_vec();
                self.cursor = self.buffer.len();
                Ok(Some(out))
            }
            "l" | "L" => match rest.iter().position(|b| *b == b'\n') {
                None => {
                    let out = rest.to_vec();
                    self.cursor = self.buffer.len();
                    Ok(Some(out))
                }
                Some(end) => {
                    let keep = if format == "L" { end + 1 } else { end };
                    let out = rest[..keep].to_vec();
                    self.cursor += end + 1;
                    Ok(Some(out))
                }
            },
            _ => Err("Invalid format".into()),
        }
    }

    /// `read(n)`, where a negative `n` rewinds and nil marks end of file.
    pub fn read_amount(&mut self, amount: i64) -> Result<Option<Vec<u8>>> {
        self.check_open()?;
        if !self.mode.can_read {
            return Err("Access denied".into());
        }
        let len = self.buffer.len() as i64;
        let target = (self.cursor as i64 + amount).clamp(0, len) as usize;
        if target == self.cursor {
            // Zero bytes asked for is an empty read; anything else here is end of file.
            return Ok((amount == 0).then(Vec::new));
        }
        let (start, end) = (target.min(self.cursor), target.max(self.cursor));
        let out = self.buffer[start..end].to_vec();
        self.cursor = target;
        Ok(Some(out))
    }

    /// Splices at the cursor and advances it.
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.check_open()?;
        if !self.mode.can_write {
            return Err("Access denied".into());
        }
        let end = (self.cursor + data.len()).min(self.buffer.len());
        self.buffer.splice(self.cursor..end, data.iter().copied());
        self.cursor += data.len();
        Ok(())
    }
}
