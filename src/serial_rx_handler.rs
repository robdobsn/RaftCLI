use crate::time_tracker::TimeTracker;
use chrono::DateTime;
use regex::Regex;

/// Processes a single line of serial data and returns the processed line.
pub fn process_line(
    line: &str,
    time_tracker: &mut TimeTracker,
    line_received_time: DateTime<chrono::Local>,
) -> String {
    line.to_string()
    // if let Some((esp32_timestamp, rest)) = extract_esp32_timestamp(line) {
    //     let augmented_timestamp = time_tracker.update(esp32_timestamp, line_received_time);
    //     format!("{} {}", augmented_timestamp, rest)
    // } else {
    //     let pc_time = time_tracker.format_pc_time(line_received_time);
    //     format!("{} {}", pc_time, line)
    // }
}

/// Extracts the ESP32 timestamp from a line of text.
/// Returns the timestamp and the rest of the line.
fn extract_esp32_timestamp(line: &str) -> Option<(u32, &str)> {
    let re = Regex::new(r"\((\d+)\)").unwrap();
    re.captures(line).and_then(|caps| {
        caps.get(1)
            .and_then(|m| m.as_str().parse::<u32>().ok())
            .map(|ts| (ts, &line[caps.get(0).unwrap().end()..]))
    })
}
