// flat_key_values.rs - RaftCLI: Flat key-value file management
// Rob Dobson 2024

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufWriter, Write};
use std::path::Path;

/// A structure to hold key-value pairs and comments from a flat key-value file.
pub struct FlatKeyValues {
    file_path: String,
    lines: Vec<String>, // Preserve the lines as they appear in the file
    data: HashMap<String, String>, // Parsed key-value pairs for quick lookups
    is_modified: bool,
}

impl FlatKeyValues {
    /// Load a flat key-value file from the given path while preserving line order, comments, and formatting.
    pub fn load_from_file(file_path: &str) -> io::Result<Self> {
        let path = Path::new(file_path);
        let file = File::open(&path)?;
        let mut data = HashMap::new();
        let mut lines = Vec::new();

        for line in io::BufReader::new(file).lines() {
            let line = line?;
            let trimmed = line.trim();

            // Add the line to the vector, regardless of its content
            lines.push(line.clone());

            // Parse key=value pairs, but don't skip comments or empty lines
            if let Some((key, value)) = trimmed.split_once('=') {
                data.insert(key.trim().to_string(), value.trim().to_string());
            }
        }

        Ok(Self {
            file_path: file_path.to_string(),
            lines,
            data,
            is_modified: false,
        })
    }

    /// Get a value by key. This will only search the already parsed data.
    pub fn get(&self, key: &str) -> Option<&String> {
        self.data.get(key)
    }

    /// Insert or update a key-value pair while preserving the file's original format.
    pub fn insert(&mut self, key: String, value: String) {
        self.is_modified = true;

        // Search for the key in the lines and update if found
        let mut key_found = false;
        for line in &mut self.lines {
            let trimmed = line.trim();
            if let Some((existing_key, _)) = trimmed.split_once('=') {
                if existing_key.trim() == key {
                    *line = format!("{} = {}", key, value); // Update the line
                    key_found = true;
                    break;
                }
            }
        }

        // If the key was not found, append it to the end
        if !key_found {
            self.lines.push(format!("{} = {}", key, value));
        }

        // Update the in-memory key-value store
        self.data.insert(key, value);
    }

    /// Save the modified file, preserving the original line order and formatting.
    pub fn save(&mut self) -> io::Result<()> {
        if !self.is_modified {
            return Ok(()); // No changes, nothing to save
        }

        // Open the file for writing (this will truncate the existing file)
        let file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(&self.file_path)?;

        let mut writer = BufWriter::new(file);

        // Write each line as it originally appeared, with comments and formatting preserved
        for line in &self.lines {
            writeln!(writer, "{}", line)?;
        }

        // Flush the writer to ensure all data is written
        writer.flush()?;

        // Reset the modification flag
        self.is_modified = false;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::FlatKeyValues;
    use std::io::Write;
    use std::fs::File;

    #[test]
    fn test_load_file() {
        // Create a temporary file
        let mut temp_file = File::create("test_sdkconfig.defaults").unwrap();
        writeln!(temp_file, "# Comment").unwrap();
        writeln!(temp_file, "KEY1 = VALUE1").unwrap();
        writeln!(temp_file, "KEY2=VALUE2").unwrap();
        writeln!(temp_file, "  KEY3  =  VALUE3  ").unwrap();
        writeln!(temp_file, "").unwrap();  // empty line
        drop(temp_file);

        let config = FlatKeyValues::load_from_file("test_sdkconfig.defaults").unwrap();

        assert_eq!(config.get("KEY1").unwrap(), "VALUE1");
        assert_eq!(config.get("KEY2").unwrap(), "VALUE2");
        assert_eq!(config.get("KEY3").unwrap(), "VALUE3");
        assert!(config.get("KEY4").is_none());
    }

    #[test]
    fn test_insert_and_save() {
        let mut config = FlatKeyValues::load_from_file("test_sdkconfig.defaults").unwrap();

        // Insert a new key-value pair
        config.insert("NEW_KEY".to_string(), "NEW_VALUE".to_string());
        assert_eq!(config.get("NEW_KEY").unwrap(), "NEW_VALUE");

        // Save the file
        config.save().unwrap();

        // Reload the file and verify the new key-value pair is there
        let new_config = FlatKeyValues::load_from_file("test_sdkconfig.defaults").unwrap();
        assert_eq!(new_config.get("NEW_KEY").unwrap(), "NEW_VALUE");

        // Clean up the test file (optional)
        std::fs::remove_file("test_sdkconfig.defaults").unwrap();
    }
}
