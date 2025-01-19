use crossterm::{
    cursor,
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
}

impl TerminalIO {
    pub fn new() -> TerminalIO {
        TerminalIO {
            command_buffer: String::new(),
            cursor_col: 0,
            cursor_row: 0,
            cols: 0,
            rows: 0,
            is_error: false,
        }
    }

    pub fn init(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let (_cols, rows) = terminal::size()?;
        self.cols = _cols;
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
        self.display_received_data(&data);

        // Get the cursor position
        let (cursor_col, mut cursor_row) = cursor::position().unwrap();

        // If the cursor is not at the first column then add a newline
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

    pub fn add_to_command_buffer(&mut self, c: char) {
        self.command_buffer.push(c);
        self.print("", false);
    }

    pub fn add_str_to_command_buffer(&mut self, s: &str) {
        self.command_buffer.push_str(s);
        self.print("", true);
    }

    pub fn backspace_command_buffer(&mut self) {
        if !self.command_buffer.is_empty() {
            self.command_buffer.pop();
            self.print("", false);
        }
    }
}
