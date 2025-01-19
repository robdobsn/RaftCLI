use crate::cmd_history::CommandHistory;
use crossterm::{
    cursor,
    event::{KeyCode, KeyEvent, KeyModifiers},
    style::{Color, ResetColor, SetForegroundColor},
    terminal::{self, ClearType},
    execute,
};
use std::io::{self, Write};

pub struct TerminalIO {
    command_buffer: String,
    cursor_col: u16,
    cursor_row: u16,
    cols: u16,
    rows: u16,
    is_error: bool,
    command_history: CommandHistory,
}

impl TerminalIO {
    pub fn new(history_file_path: &str) -> TerminalIO {
        TerminalIO {
            command_buffer: String::new(),
            cursor_col: 0,
            cursor_row: 0,
            cols: 0,
            rows: 0,
            is_error: false,
            command_history: CommandHistory::new(history_file_path),
        }
    }

    pub fn init(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let (cols, rows) = terminal::size()?;
        self.cols = cols;
        self.rows = rows;

        // Setup terminal for raw mode
        terminal::enable_raw_mode()?;
        execute!(
            io::stdout(),
            terminal::Clear(ClearType::All),
            cursor::MoveTo(0, 0)
        )?;
        Ok(())
    }

    pub fn handle_key_event(
        &mut self,
        key_event: KeyEvent,
        send_command: impl FnOnce(String),
    ) -> bool {
        match key_event.code {
            KeyCode::Char('c') | KeyCode::Char('x')
                if key_event.modifiers == KeyModifiers::CONTROL =>
            {
                return false;
            }
            KeyCode::Esc => return false,
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
                self.add_char_to_command_buffer(c);
            }
            KeyCode::Up => {
                // Move up first
                self.command_history.move_up();
                // Now get current
                let current_command = self.command_history.get_current().clone();
                // Set the command buffer to the current command
                self.set_command_buffer(current_command);
            }
            KeyCode::Down => {
                // Move down first
                self.command_history.move_down();
                // Now get current
                let current_command = self.command_history.get_current().clone();
                // Set the command buffer to the current command
                self.set_command_buffer(current_command);
            }
            _ => {}
        }
        true
    }
    
    pub fn print(&mut self, data: &str, force_show: bool) {
        if !force_show && self.is_error {
            return;
        }

        // Clear error flag
        self.is_error = false;

        // Clear the last line of the terminal (command buffer)
        execute!(
            io::stdout(),
            cursor::MoveTo(0, self.rows - 1),
            terminal::Clear(ClearType::CurrentLine)
        )
        .unwrap();

        // Move the cursor to the position of the last output
        execute!(
            io::stdout(),
            cursor::MoveTo(self.cursor_col, self.cursor_row)
        )
        .unwrap();

        // Display the received data
        self.display_received_data(data);

        // Get the cursor position
        let (cursor_col, mut cursor_row) = cursor::position().unwrap();

        // If the cursor is not at the first column, add a newline
        if cursor_col != 0 && cursor_row == self.rows - 1 {
            print!("\n");
            cursor_row -= 1;
        }

        // Save the cursor position
        self.cursor_col = cursor_col;
        self.cursor_row = cursor_row;

        // Move the cursor to the bottom line and clear it
        execute!(
            io::stdout(),
            cursor::MoveTo(0, self.rows - 1),
            terminal::Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::Yellow),
        )
        .unwrap();

        // Display the command buffer
        print!("> {}", self.command_buffer);

        // Reset the text color
        execute!(io::stdout(), ResetColor).unwrap();

        // Flush the output
        io::stdout().flush().unwrap();
    }

    pub fn show_error(&mut self, error_msg: &str) {
        // Move the cursor to the bottom line and clear it
        execute!(
            io::stdout(),
            cursor::MoveTo(0, self.rows - 1),
            terminal::Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::Red),
        )
        .unwrap();

        // Display the error message
        print!("! {}", error_msg);

        // Reset the text color
        execute!(io::stdout(), ResetColor).unwrap();

        // Flush the output
        io::stdout().flush().unwrap();

        // Set the error flag
        self.is_error = true;
    }

    pub fn show_info(&mut self, info_msg: &str) {
        // Move the cursor to the bottom line and clear it
        execute!(
            io::stdout(),
            cursor::MoveTo(0, self.rows - 1),
            terminal::Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::Green),
        )
        .unwrap();

        // Display the info message
        print!("> {}", info_msg);

        // Reset the text color
        execute!(io::stdout(), ResetColor).unwrap();

        // Flush the output
        io::stdout().flush().unwrap();
    }

    pub fn clear_info(&mut self) {
        self.set_command_buffer("".to_string());
    }

    pub fn display_received_data(&mut self, data: &str) {
        print!("{}", data);
        io::stdout().flush().unwrap();
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
