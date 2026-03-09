use crate::cmd_history::CommandHistory;
use crate::native_terminal::{self, KeyCode, KeyEvent, NativeTerminal};


pub struct TerminalIO {
    command_buffer: String,
    cursor_col: u16,
    cursor_row: u16,
    cols: u16,
    rows: u16,
    is_error: bool,
    command_history: CommandHistory,
    terminal: NativeTerminal,
}

impl TerminalIO {
    pub fn new(history_file_path: &str) -> TerminalIO {
        let terminal = NativeTerminal::new().expect("Failed to initialize terminal");
        TerminalIO {
            command_buffer: String::new(),
            cursor_col: 0,
            cursor_row: 0,
            cols: 0,
            rows: 0,
            is_error: false,
            command_history: CommandHistory::new(history_file_path),
            terminal,
        }
    }

    pub fn init(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let (cols, rows) = self.terminal.size();
        self.cols = cols;
        self.rows = rows;

        self.terminal.clear_screen();

        // Set scroll region to all rows except the last (prompt row)
        if self.rows > 1 {
            self.terminal.set_scroll_region(0, self.rows - 2);
        }
        self.terminal.move_to(0, 0);
        Ok(())
    }

    pub fn cleanup(&mut self) {
        self.terminal.cleanup();
    }

    pub fn handle_key_event(
        &mut self,
        key_event: KeyEvent,
        send_command: impl FnOnce(String),
    ) -> bool {
        match &key_event.code {
            KeyCode::Char('c') | KeyCode::Char('x') if key_event.modifiers.ctrl => {
                return false;
            }
            KeyCode::Escape => return false,
            KeyCode::Enter => {
                let command = self.get_command_buffer();
                send_command(command.clone());
                self.command_history.add_command(&command);
                self.clear_command_buffer();
            }
            KeyCode::Backspace => {
                self.backspace_command_buffer();
            }
            KeyCode::Char(c) => {
                self.add_char_to_command_buffer(*c);
            }
            KeyCode::Up => {
                self.command_history.move_up();
                let current_command = self.command_history.get_current().clone();
                self.set_command_buffer(current_command);
            }
            KeyCode::Down => {
                self.command_history.move_down();
                let current_command = self.command_history.get_current().clone();
                self.set_command_buffer(current_command);
            }
            _ => {}
        }
        true
    }

    /// Poll for a terminal event with the given timeout.
    pub fn poll_event(&mut self, timeout: std::time::Duration) -> bool {
        self.terminal.poll_event(timeout)
    }

    /// Read the next terminal event.
    pub fn read_event(&mut self) -> Option<native_terminal::TermEvent> {
        self.terminal.read_event()
    }

    pub fn handle_resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        // Clamp cursor to new bounds
        if self.cursor_row >= rows.saturating_sub(1) {
            self.cursor_row = rows.saturating_sub(2);
        }
        // Update scroll region
        if self.rows > 1 {
            self.terminal.set_scroll_region(0, self.rows - 2);
        }
        self.draw_prompt();
    }
    
    pub fn print(&mut self, data: &str, force_show: bool) {
        if !force_show && self.is_error {
            return;
        }

        // Clear error flag
        self.is_error = false;

        // Clear the prompt line
        self.terminal.move_to(0, self.rows - 1);
        self.terminal.clear_line();

        // Move into the scroll region to the saved output position
        self.terminal.move_to(self.cursor_col, self.cursor_row);

        // Set scroll region so the terminal handles scrolling within the output area
        if self.rows > 1 {
            self.terminal.set_scroll_region(0, self.rows - 2);
        }
        self.terminal.move_to(self.cursor_col, self.cursor_row);

        // Display the received data — let the terminal scroll naturally
        if !data.is_empty() {
            self.display_received_data(data);
        }

        // After printing, we need to know where the cursor ended up.
        // Since the terminal handles scrolling natively, we track position
        // by counting what we wrote.
        self.update_cursor_after_print(data);

        // Draw the prompt on the fixed bottom row
        self.draw_prompt();
    }

    fn update_cursor_after_print(&mut self, data: &str) {
        let max_row = self.rows.saturating_sub(2);
        for ch in data.chars() {
            match ch {
                '\n' => {
                    self.cursor_col = 0;
                    if self.cursor_row < max_row {
                        self.cursor_row += 1;
                    }
                    // If at max_row, the terminal scrolled — row stays
                }
                '\r' => {
                    self.cursor_col = 0;
                }
                c if !c.is_control() => {
                    self.cursor_col += 1;
                    if self.cursor_col >= self.cols {
                        self.cursor_col = 0;
                        if self.cursor_row < max_row {
                            self.cursor_row += 1;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn draw_prompt(&mut self) {
        // Temporarily reset scroll region so we can write on the last row
        self.terminal.reset_scroll_region();
        self.terminal.move_to(0, self.rows - 1);
        self.terminal.clear_line();
        self.terminal.set_color_yellow();
        self.terminal.write_str(&format!("> {}", self.command_buffer));
        self.terminal.reset_color();
        // Restore scroll region
        if self.rows > 1 {
            self.terminal.set_scroll_region(0, self.rows - 2);
        }
        self.terminal.flush();
    }

    pub fn show_error(&mut self, error_msg: &str) {
        self.terminal.reset_scroll_region();
        self.terminal.move_to(0, self.rows - 1);
        self.terminal.clear_line();
        self.terminal.set_color_red();
        self.terminal.write_str(&format!("! {}", error_msg));
        self.terminal.reset_color();
        self.terminal.flush();
        if self.rows > 1 {
            self.terminal.set_scroll_region(0, self.rows - 2);
        }
        self.is_error = true;
    }

    pub fn show_info(&mut self, info_msg: &str) {
        self.terminal.reset_scroll_region();
        self.terminal.move_to(0, self.rows - 1);
        self.terminal.clear_line();
        self.terminal.set_color_green();
        self.terminal.write_str(&format!("> {}", info_msg));
        self.terminal.reset_color();
        self.terminal.flush();
        if self.rows > 1 {
            self.terminal.set_scroll_region(0, self.rows - 2);
        }
    }

    pub fn clear_info(&mut self) {
        self.set_command_buffer("".to_string());
    }

    pub fn display_received_data(&mut self, data: &str) {
        self.terminal.write_str(data);
    }

    pub fn get_command_buffer(&self) -> String {
        self.command_buffer.clone()
    }

    pub fn clear_command_buffer(&mut self) {
        self.command_buffer.clear();
        self.print("", false);
    }

    pub fn add_char_to_command_buffer(&mut self, c: char) {
        self.command_buffer.push(c);
        self.print("", false);
    }

    pub fn set_command_buffer(&mut self, s: String) {
        self.command_buffer = s;
        self.print("", true);
    }

    pub fn backspace_command_buffer(&mut self) {
        if !self.command_buffer.is_empty() {
            self.command_buffer.pop();
            self.print("", false);
        }
    }
}
