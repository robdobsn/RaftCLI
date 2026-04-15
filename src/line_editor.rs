// RaftCLI: Line editor module
// Rob Dobson 2024-2026
//
// Bash-style line editing: cursor movement, insert/overwrite, word operations,
// kill/yank, and command history. Independent of terminal I/O — callers query
// buffer_str() and cursor_pos() to render the prompt.

use crate::cmd_history::CommandHistory;
use crate::native_terminal::{KeyCode, KeyEvent};

/// Result of handling a key event
pub enum LineEditAction {
    /// Buffer or cursor changed — caller should redraw the prompt
    Updated,
    /// User submitted a command (Enter)
    Submit(String),
    /// User requested exit (Ctrl+C, Ctrl+X, Escape)
    Exit,
    /// Key was not handled / no change
    None,
}

pub struct LineEditor {
    buf: Vec<char>,
    cursor: usize,
    insert_mode: bool,
    history: CommandHistory,
}

impl LineEditor {
    pub fn new(history_file_path: &str) -> Self {
        Self {
            buf: Vec::new(),
            cursor: 0,
            insert_mode: true,
            history: CommandHistory::new(history_file_path),
        }
    }

    /// The current command buffer as a String.
    pub fn buffer_str(&self) -> String {
        self.buf.iter().collect()
    }

    /// Cursor position in characters (0 = before first char).
    pub fn cursor_pos(&self) -> usize {
        self.cursor
    }

    /// Whether the editor is in insert mode (true) or overwrite mode (false).
    #[cfg(test)]
    pub fn is_insert_mode(&self) -> bool {
        self.insert_mode
    }

    /// Process a key event and return what happened.
    pub fn handle_key(&mut self, key: &KeyEvent) -> LineEditAction {
        match &key.code {
            // ── Exit ──
            KeyCode::Char('c') | KeyCode::Char('x') if key.modifiers.ctrl => LineEditAction::Exit,
            KeyCode::Escape => LineEditAction::Exit,

            // ── Submit ──
            KeyCode::Enter => {
                let command = self.buffer_str();
                self.history.add_command(&command);
                self.buf.clear();
                self.cursor = 0;
                LineEditAction::Submit(command)
            }

            // ── Cursor movement ──
            KeyCode::Left if key.modifiers.ctrl => {
                self.cursor = self.prev_word_boundary();
                LineEditAction::Updated
            }
            KeyCode::Right if key.modifiers.ctrl => {
                self.cursor = self.next_word_boundary();
                LineEditAction::Updated
            }
            KeyCode::Left => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                LineEditAction::Updated
            }
            KeyCode::Right => {
                if self.cursor < self.buf.len() {
                    self.cursor += 1;
                }
                LineEditAction::Updated
            }
            KeyCode::Home | KeyCode::Char('a') if key.code == KeyCode::Home || key.modifiers.ctrl => {
                self.cursor = 0;
                LineEditAction::Updated
            }
            KeyCode::End | KeyCode::Char('e') if key.code == KeyCode::End || key.modifiers.ctrl => {
                self.cursor = self.buf.len();
                LineEditAction::Updated
            }

            // ── Insert/Overwrite toggle ──
            KeyCode::Insert => {
                self.insert_mode = !self.insert_mode;
                LineEditAction::Updated
            }

            // ── Deletion ──
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.buf.remove(self.cursor);
                    LineEditAction::Updated
                } else {
                    LineEditAction::None
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.buf.len() {
                    self.buf.remove(self.cursor);
                    LineEditAction::Updated
                } else {
                    LineEditAction::None
                }
            }

            // ── Kill operations ──
            KeyCode::Char('k') if key.modifiers.ctrl => {
                // Kill to end of line
                self.buf.truncate(self.cursor);
                LineEditAction::Updated
            }
            KeyCode::Char('u') if key.modifiers.ctrl => {
                // Kill to start of line
                self.buf.drain(..self.cursor);
                self.cursor = 0;
                LineEditAction::Updated
            }
            KeyCode::Char('w') if key.modifiers.ctrl => {
                // Kill previous word
                let new_pos = self.prev_word_boundary();
                self.buf.drain(new_pos..self.cursor);
                self.cursor = new_pos;
                LineEditAction::Updated
            }

            // ── History ──
            KeyCode::Up => {
                self.history.move_up();
                self.set_buffer(&self.history.get_current());
                LineEditAction::Updated
            }
            KeyCode::Down => {
                self.history.move_down();
                self.set_buffer(&self.history.get_current());
                LineEditAction::Updated
            }

            // ── Character input ──
            KeyCode::Char(c) => {
                if self.insert_mode || self.cursor >= self.buf.len() {
                    self.buf.insert(self.cursor, *c);
                } else {
                    self.buf[self.cursor] = *c;
                }
                self.cursor += 1;
                LineEditAction::Updated
            }

            _ => LineEditAction::None,
        }
    }

    // ── Helpers ──

    fn set_buffer(&mut self, s: &str) {
        self.buf = s.chars().collect();
        self.cursor = self.buf.len();
    }

    fn prev_word_boundary(&self) -> usize {
        if self.cursor == 0 {
            return 0;
        }
        let mut pos = self.cursor - 1;
        // Skip whitespace
        while pos > 0 && self.buf[pos].is_whitespace() {
            pos -= 1;
        }
        // Skip word characters
        while pos > 0 && !self.buf[pos - 1].is_whitespace() {
            pos -= 1;
        }
        pos
    }

    fn next_word_boundary(&self) -> usize {
        let len = self.buf.len();
        if self.cursor >= len {
            return len;
        }
        let mut pos = self.cursor;
        // Skip current word characters
        while pos < len && !self.buf[pos].is_whitespace() {
            pos += 1;
        }
        // Skip whitespace
        while pos < len && self.buf[pos].is_whitespace() {
            pos += 1;
        }
        pos
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_terminal::Modifiers;

    fn make_editor() -> LineEditor {
        // Use a non-existent path so no file I/O happens during tests
        LineEditor::new("")
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: Modifiers::default(),
        }
    }

    fn ctrl_key(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: Modifiers { ctrl: true },
        }
    }

    fn ctrl_arrow(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: Modifiers { ctrl: true },
        }
    }

    fn type_str(editor: &mut LineEditor, s: &str) {
        for c in s.chars() {
            editor.handle_key(&key(KeyCode::Char(c)));
        }
    }

    // ── Basic typing ──

    #[test]
    fn test_basic_typing() {
        let mut ed = make_editor();
        type_str(&mut ed, "hello");
        assert_eq!(ed.buffer_str(), "hello");
        assert_eq!(ed.cursor_pos(), 5);
    }

    #[test]
    fn test_submit_clears_buffer() {
        let mut ed = make_editor();
        type_str(&mut ed, "cmd");
        match ed.handle_key(&key(KeyCode::Enter)) {
            LineEditAction::Submit(s) => assert_eq!(s, "cmd"),
            _ => panic!("expected Submit"),
        }
        assert_eq!(ed.buffer_str(), "");
        assert_eq!(ed.cursor_pos(), 0);
    }

    // ── Cursor movement ──

    #[test]
    fn test_left_right() {
        let mut ed = make_editor();
        type_str(&mut ed, "abcde");
        ed.handle_key(&key(KeyCode::Left));
        ed.handle_key(&key(KeyCode::Left));
        assert_eq!(ed.cursor_pos(), 3);
        ed.handle_key(&key(KeyCode::Right));
        assert_eq!(ed.cursor_pos(), 4);
    }

    #[test]
    fn test_left_at_start_stays() {
        let mut ed = make_editor();
        type_str(&mut ed, "ab");
        ed.handle_key(&key(KeyCode::Left));
        ed.handle_key(&key(KeyCode::Left));
        ed.handle_key(&key(KeyCode::Left)); // should not go below 0
        assert_eq!(ed.cursor_pos(), 0);
    }

    #[test]
    fn test_right_at_end_stays() {
        let mut ed = make_editor();
        type_str(&mut ed, "ab");
        ed.handle_key(&key(KeyCode::Right)); // already at end
        assert_eq!(ed.cursor_pos(), 2);
    }

    #[test]
    fn test_home_end() {
        let mut ed = make_editor();
        type_str(&mut ed, "hello world");
        ed.handle_key(&key(KeyCode::Home));
        assert_eq!(ed.cursor_pos(), 0);
        ed.handle_key(&key(KeyCode::End));
        assert_eq!(ed.cursor_pos(), 11);
    }

    #[test]
    fn test_ctrl_a_e() {
        let mut ed = make_editor();
        type_str(&mut ed, "hello");
        ed.handle_key(&ctrl_key('a'));
        assert_eq!(ed.cursor_pos(), 0);
        ed.handle_key(&ctrl_key('e'));
        assert_eq!(ed.cursor_pos(), 5);
    }

    // ── Insert in the middle ──

    #[test]
    fn test_insert_in_middle() {
        let mut ed = make_editor();
        type_str(&mut ed, "hllo");
        ed.handle_key(&key(KeyCode::Home));
        ed.handle_key(&key(KeyCode::Right)); // cursor at 1
        ed.handle_key(&key(KeyCode::Char('e')));
        assert_eq!(ed.buffer_str(), "hello");
        assert_eq!(ed.cursor_pos(), 2);
    }

    // ── Overwrite mode ──

    #[test]
    fn test_overwrite_mode() {
        let mut ed = make_editor();
        type_str(&mut ed, "abcde");
        ed.handle_key(&key(KeyCode::Insert)); // switch to overwrite
        assert!(!ed.is_insert_mode());
        ed.handle_key(&key(KeyCode::Home));
        ed.handle_key(&key(KeyCode::Char('X')));
        ed.handle_key(&key(KeyCode::Char('Y')));
        assert_eq!(ed.buffer_str(), "XYcde");
        assert_eq!(ed.cursor_pos(), 2);
    }

    #[test]
    fn test_overwrite_at_end_appends() {
        let mut ed = make_editor();
        type_str(&mut ed, "ab");
        ed.handle_key(&key(KeyCode::Insert)); // overwrite
        ed.handle_key(&key(KeyCode::Char('c')));
        assert_eq!(ed.buffer_str(), "abc");
    }

    // ── Backspace and Delete ──

    #[test]
    fn test_backspace_at_end() {
        let mut ed = make_editor();
        type_str(&mut ed, "hello");
        ed.handle_key(&key(KeyCode::Backspace));
        assert_eq!(ed.buffer_str(), "hell");
        assert_eq!(ed.cursor_pos(), 4);
    }

    #[test]
    fn test_backspace_in_middle() {
        let mut ed = make_editor();
        type_str(&mut ed, "hello");
        ed.handle_key(&key(KeyCode::Left));
        ed.handle_key(&key(KeyCode::Left));
        ed.handle_key(&key(KeyCode::Backspace)); // delete 'l' at pos 2
        assert_eq!(ed.buffer_str(), "helo");
        assert_eq!(ed.cursor_pos(), 2);
    }

    #[test]
    fn test_backspace_at_start_does_nothing() {
        let mut ed = make_editor();
        type_str(&mut ed, "ab");
        ed.handle_key(&key(KeyCode::Home));
        ed.handle_key(&key(KeyCode::Backspace));
        assert_eq!(ed.buffer_str(), "ab");
        assert_eq!(ed.cursor_pos(), 0);
    }

    #[test]
    fn test_delete_forward() {
        let mut ed = make_editor();
        type_str(&mut ed, "hello");
        ed.handle_key(&key(KeyCode::Home));
        ed.handle_key(&key(KeyCode::Delete));
        assert_eq!(ed.buffer_str(), "ello");
        assert_eq!(ed.cursor_pos(), 0);
    }

    #[test]
    fn test_delete_at_end_does_nothing() {
        let mut ed = make_editor();
        type_str(&mut ed, "hello");
        ed.handle_key(&key(KeyCode::Delete));
        assert_eq!(ed.buffer_str(), "hello");
    }

    // ── Kill operations ──

    #[test]
    fn test_ctrl_k_kill_to_end() {
        let mut ed = make_editor();
        type_str(&mut ed, "hello world");
        ed.handle_key(&key(KeyCode::Home));
        ed.handle_key(&key(KeyCode::Right)); // pos 1
        ed.handle_key(&ctrl_key('k'));
        assert_eq!(ed.buffer_str(), "h");
        assert_eq!(ed.cursor_pos(), 1);
    }

    #[test]
    fn test_ctrl_u_kill_to_start() {
        let mut ed = make_editor();
        type_str(&mut ed, "hello world");
        // cursor at 11 (end)
        ed.handle_key(&key(KeyCode::Left)); // 10
        ed.handle_key(&key(KeyCode::Left)); // 9
        ed.handle_key(&key(KeyCode::Left)); // 8
        ed.handle_key(&key(KeyCode::Left)); // 7
        ed.handle_key(&key(KeyCode::Left)); // 6 — at 'w'
        ed.handle_key(&ctrl_key('u'));
        assert_eq!(ed.buffer_str(), "world");
        assert_eq!(ed.cursor_pos(), 0);
    }

    #[test]
    fn test_ctrl_w_kill_word() {
        let mut ed = make_editor();
        type_str(&mut ed, "one two three");
        ed.handle_key(&ctrl_key('w')); // kill "three"
        assert_eq!(ed.buffer_str(), "one two ");
        ed.handle_key(&ctrl_key('w')); // kill "two "
        assert_eq!(ed.buffer_str(), "one ");
    }

    // ── Word movement ──

    #[test]
    fn test_ctrl_left_word_jump() {
        let mut ed = make_editor();
        type_str(&mut ed, "one two three");
        ed.handle_key(&ctrl_arrow(KeyCode::Left));
        assert_eq!(ed.cursor_pos(), 8); // start of "three"
        ed.handle_key(&ctrl_arrow(KeyCode::Left));
        assert_eq!(ed.cursor_pos(), 4); // start of "two"
        ed.handle_key(&ctrl_arrow(KeyCode::Left));
        assert_eq!(ed.cursor_pos(), 0); // start of "one"
    }

    #[test]
    fn test_ctrl_right_word_jump() {
        let mut ed = make_editor();
        type_str(&mut ed, "one two three");
        ed.handle_key(&key(KeyCode::Home));
        ed.handle_key(&ctrl_arrow(KeyCode::Right));
        assert_eq!(ed.cursor_pos(), 4); // after "one "
        ed.handle_key(&ctrl_arrow(KeyCode::Right));
        assert_eq!(ed.cursor_pos(), 8); // after "two "
        ed.handle_key(&ctrl_arrow(KeyCode::Right));
        assert_eq!(ed.cursor_pos(), 13); // end
    }

    // ── History ──

    #[test]
    fn test_history_up_down() {
        let mut ed = make_editor();
        type_str(&mut ed, "first");
        ed.handle_key(&key(KeyCode::Enter));
        type_str(&mut ed, "second");
        ed.handle_key(&key(KeyCode::Enter));

        ed.handle_key(&key(KeyCode::Up));
        assert_eq!(ed.buffer_str(), "second");
        ed.handle_key(&key(KeyCode::Up));
        assert_eq!(ed.buffer_str(), "first");
        ed.handle_key(&key(KeyCode::Down));
        assert_eq!(ed.buffer_str(), "second");
        ed.handle_key(&key(KeyCode::Down));
        assert_eq!(ed.buffer_str(), "");
    }

    #[test]
    fn test_history_sets_cursor_to_end() {
        let mut ed = make_editor();
        type_str(&mut ed, "hello");
        ed.handle_key(&key(KeyCode::Enter));
        ed.handle_key(&key(KeyCode::Up));
        assert_eq!(ed.cursor_pos(), 5);
    }

    // ── Exit ──

    #[test]
    fn test_ctrl_c_exits() {
        let mut ed = make_editor();
        match ed.handle_key(&ctrl_key('c')) {
            LineEditAction::Exit => {}
            _ => panic!("expected Exit"),
        }
    }

    #[test]
    fn test_escape_exits() {
        let mut ed = make_editor();
        match ed.handle_key(&key(KeyCode::Escape)) {
            LineEditAction::Exit => {}
            _ => panic!("expected Exit"),
        }
    }

    // ── Edge cases ──

    #[test]
    fn test_empty_submit() {
        let mut ed = make_editor();
        match ed.handle_key(&key(KeyCode::Enter)) {
            LineEditAction::Submit(s) => assert_eq!(s, ""),
            _ => panic!("expected Submit"),
        }
    }

    #[test]
    fn test_utf8_characters() {
        let mut ed = make_editor();
        type_str(&mut ed, "café");
        assert_eq!(ed.buffer_str(), "café");
        assert_eq!(ed.cursor_pos(), 4);
        ed.handle_key(&key(KeyCode::Backspace));
        assert_eq!(ed.buffer_str(), "caf");
        ed.handle_key(&key(KeyCode::Home));
        ed.handle_key(&key(KeyCode::Delete));
        assert_eq!(ed.buffer_str(), "af");
    }

    #[test]
    fn test_insert_toggle_back() {
        let mut ed = make_editor();
        assert!(ed.is_insert_mode());
        ed.handle_key(&key(KeyCode::Insert));
        assert!(!ed.is_insert_mode());
        ed.handle_key(&key(KeyCode::Insert));
        assert!(ed.is_insert_mode());
    }
}
