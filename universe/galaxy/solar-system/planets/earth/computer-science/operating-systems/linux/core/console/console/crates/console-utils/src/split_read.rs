use std::io::{BufRead, ErrorKind};

use memchr::memmem::Finder;

pub struct SplitRead<R> {
    reader: Option<R>,
    buf: Option<Vec<u8>>,
    finder: Finder<'static>,
}

impl<R: BufRead> SplitRead<R> {
    pub fn new(reader: R, delim: impl AsRef<[u8]>) -> Self {
        // empty delimiter results in an infinite stream of empty items
        debug_assert!(!delim.as_ref().is_empty(), "delimiter can't be empty");
        Self {
            reader: Some(reader),
            buf: Some(Vec::new()),
            finder: Finder::new(delim.as_ref()).into_owned(),
        }
    }
}

impl<R: BufRead> Iterator for SplitRead<R> {
    type Item = Result<Vec<u8>, std::io::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let buf = self.buf.as_mut()?;
        let mut search_start = 0usize;

        loop {
            if let Some(i) = self.finder.find(&buf[search_start..]) {
                let needle_idx = search_start + i;
                let right = buf.split_off(needle_idx + self.finder.needle().len());
                buf.truncate(needle_idx);
                let left = std::mem::replace(buf, right);
                return Some(Ok(left));
            }

            if let Some(mut r) = self.reader.take() {
                search_start = buf.len().saturating_sub(self.finder.needle().len() + 1);
                let available = match r.fill_buf() {
                    Ok(n) => n,
                    Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => return Some(Err(e)),
                };

                buf.extend_from_slice(available);
                let used = available.len();
                r.consume(used);
                if used != 0 {
                    self.reader = Some(r);
                }
                continue;
            } else {
                return self.buf.take().map(Ok);
            }
        }
    }
}

