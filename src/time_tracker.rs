use chrono::DateTime;

// pub struct TimeTracker {
//     current_offset: Option<std::time::Duration>,
//     last_esp32_timestamp: Option<u32>,
//     last_pc_time: Option<std::time::Instant>,
//     reference_time: chrono::DateTime<chrono::Local>, // Wall-clock time when program starts
//     start_time: std::time::Instant,                 // Monotonic clock start time
// }
pub struct TimeTracker;

impl TimeTracker {
    pub fn new() -> Self {
        Self 
        // {
            // current_offset: None,
            // last_esp32_timestamp: None,
            // last_pc_time: None,
            // reference_time: chrono::Local::now(),
            // start_time: std::time::Instant::now(),
        // }
    }

    pub fn update(&mut self, esp32_timestamp: u32, line_received_time: DateTime<chrono::Local>) -> String {
        // if let Some(last_esp32) = self.last_esp32_timestamp {
        //     if esp32_timestamp < last_esp32 {
        //         // Restart tracking
        //         self.current_offset = Some(line_received_time - self.start_time);
        //     } else {
        //         // Update tracking with the minimum discrepancy
        //         let new_offset = (line_received_time - self.start_time)
        //             .checked_sub(std::time::Duration::from_millis(esp32_timestamp as u64))
        //             .unwrap_or_default();
        //         self.current_offset = Some(match self.current_offset {
        //             Some(current) => std::cmp::min(current, new_offset),
        //             None => new_offset,
        //         });
        //     }
        // } else {
        //     // Initialize tracking
        //     self.current_offset = Some(line_received_time - self.start_time);
        // }

        // self.last_esp32_timestamp = Some(esp32_timestamp);
        // self.last_pc_time = Some(line_received_time);

        self.format_timestamp(esp32_timestamp, line_received_time)
        // self.format_pc_time(line_received_time)
    }

    fn format_timestamp(&self, esp32_timestamp: u32, line_received_time: DateTime<chrono::Local>) -> String {
        // let pc_time = self.get_wall_clock_time(line_received_time);
        // if let Some(_offset) = self.current_offset {
        //     let adjusted_time = pc_time - chrono::Duration::milliseconds(esp32_timestamp as i64);
        //     let formatted_pc_time = adjusted_time.format("%H:%M:%S%.6f").to_string();
        //     format!("({} {})", esp32_timestamp, formatted_pc_time)
        // } else {
        //     format!("({})", esp32_timestamp)
        // }
        format!("({} {})", esp32_timestamp, self.format_pc_time(line_received_time))
    }

    // fn get_wall_clock_time(&self, line_received_time: std::time::Instant) -> chrono::DateTime<chrono::Local> {
    //     let elapsed = line_received_time - self.start_time;
    //     self.reference_time + chrono::Duration::from_std(elapsed).unwrap_or_else(|_| chrono::Duration::zero())
    // }

    pub fn format_pc_time(&self, line_received_time: DateTime<chrono::Local>) -> String {
        // let pc_time = self.get_wall_clock_time(line_received_time);
        // pc_time.format("%H:%M:%S%.6f").to_string()
        // let now = chrono::Local::now();
        // let elapsed = line_received_time.elapsed().as_micros() % 1_000_000; // Microseconds precision
        // now.format("%H:%M:%S").to_string() + &format!(".{:06}", elapsed)        
        line_received_time.format("%H:%M:%S%.6f").to_string()
    }
}
