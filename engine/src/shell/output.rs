//! Bounded byte replay shared by PTY polling and live transports.
use std::collections::VecDeque;

pub const MAX_OUTPUT_BYTES: usize = 1_048_576;

/// PTY reads can split a Unicode code point. Raw xterm bytes are unchanged;
/// this decoder keeps transcript text correct across those read boundaries.
#[derive(Default)]
pub struct TranscriptDecoder {
    pending: Vec<u8>,
}

impl TranscriptDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut text = String::new();
        let mut consumed = 0;
        loop {
            match std::str::from_utf8(&self.pending[consumed..]) {
                Ok(valid) => {
                    text.push_str(valid);
                    consumed = self.pending.len();
                    break;
                }
                Err(error) => {
                    let valid_end = consumed + error.valid_up_to();
                    text.push_str(std::str::from_utf8(&self.pending[consumed..valid_end]).unwrap());
                    consumed = valid_end;
                    match error.error_len() {
                        Some(length) => {
                            text.push('\u{fffd}');
                            consumed += length;
                        }
                        None => break,
                    }
                }
            }
        }
        self.pending.drain(..consumed);
        text
    }

    pub fn finish(&mut self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut self.pending)).into_owned()
    }
}

#[derive(Clone, Debug)]
pub struct OutputFrame {
    pub sequence: u64,
    pub data: Vec<u8>,
}

#[derive(Default)]
pub struct OutputHistory {
    frames: VecDeque<OutputFrame>,
    bytes: usize,
    sequence: u64,
    discarded_through: u64,
    pub poll_cursor: u64,
}

impl OutputHistory {
    pub fn push(&mut self, data: Vec<u8>) -> OutputFrame {
        self.sequence += 1;
        let frame = OutputFrame {
            sequence: self.sequence,
            data,
        };
        self.bytes += frame.data.len();
        self.frames.push_back(frame.clone());
        while self.bytes > MAX_OUTPUT_BYTES {
            if let Some(removed) = self.frames.pop_front() {
                self.bytes -= removed.data.len();
                self.discarded_through = removed.sequence;
            }
        }
        frame
    }

    pub fn replay(&self, after: u64) -> (Vec<OutputFrame>, bool) {
        (
            self.frames
                .iter()
                .filter(|frame| frame.sequence > after)
                .cloned()
                .collect(),
            after < self.discarded_through,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_decoder_preserves_split_unicode_and_marks_invalid_bytes() {
        let mut decoder = TranscriptDecoder::default();
        assert_eq!(decoder.push(&[b'a', 0xe7]), "a");
        assert_eq!(decoder.push(&[0x95]), "");
        assert_eq!(decoder.push(&[0x8c, 0xff, 0xe7]), "界\u{fffd}");
        assert_eq!(decoder.finish(), "\u{fffd}");
        assert_eq!(decoder.finish(), "");
    }

    #[test]
    fn replay_is_bounded_latest_bytes_and_reports_loss() {
        let mut history = OutputHistory::default();
        for _ in 0..300 {
            history.push(vec![42; 4096]);
        }
        let (frames, truncated) = history.replay(0);
        assert!(truncated);
        assert_eq!(frames.last().unwrap().sequence, 300);
        assert!(frames.iter().map(|frame| frame.data.len()).sum::<usize>() <= MAX_OUTPUT_BYTES);
        assert!(history.replay(300).0.is_empty());
        let (tail, lost) = history.replay(299);
        assert!(!lost);
        assert_eq!(tail.len(), 1);
    }
}
