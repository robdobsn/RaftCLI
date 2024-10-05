// RaftCLI: Command History Module
// Rob Dobson 2024

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

pub struct CommandHistory {
    history: Vec<String>,
    position: usize,
    history_file_path: String,
}

impl CommandHistory {
    pub fn new(history_file_path: &str) -> CommandHistory {
        let mut history = Vec::new();

        // Load history from the file if it exists
        if Path::new(history_file_path).exists() {
            if let Ok(file) = File::open(history_file_path) {
                let reader = BufReader::new(file);
                for line in reader.lines() {
                    if let Ok(command) = line {
                        history.push(command);
                    }
                }
            }
        }

        let position = history.len();

        CommandHistory {
            history,
            position,
            history_file_path: history_file_path.to_string(),
        }
    }

    pub fn add_command(&mut self, command: &str) {
        if !command.is_empty() {
            // Avoid duplicate consecutive entries
            if self.history.is_empty() || self.history.last().unwrap() != command {
                self.history.push(command.to_string());
                self.position = self.history.len();

                // Append command to history file
                if let Ok(mut file) = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.history_file_path)
                {
                    writeln!(file, "{}", command).unwrap();
                }
            }
        }
    }

    pub fn get_previous(&mut self) -> Option<&str> {
        if self.position > 0 {
            self.position -= 1;
            Some(&self.history[self.position])
        } else {
            None
        }
    }

    pub fn get_next(&mut self) -> Option<&str> {
        if self.position < self.history.len() {
            self.position += 1;
        }
        if self.position < self.history.len() {
            Some(&self.history[self.position])
        } else {
            self.position = self.history.len();
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_command_history() {
        let test_history_path = "test_raftcli_history.txt";
        let _ = fs::remove_file(test_history_path);

        let mut command_history = CommandHistory::new(test_history_path);

        command_history.add_command("first command");
        command_history.add_command("second command");
        command_history.add_command("third command");

        assert_eq!(command_history.get_previous(), Some("third command"));
        assert_eq!(command_history.get_previous(), Some("second command"));
        assert_eq!(command_history.get_previous(), Some("first command"));
        assert_eq!(command_history.get_previous(), None);

        assert_eq!(command_history.get_next(), Some("second command"));
        assert_eq!(command_history.get_next(), Some("third command"));
        assert_eq!(command_history.get_next(), None);
        assert_eq!(command_history.get_next(), None);

        // Cleanup
        let _ = fs::remove_file(test_history_path);
    }
}
