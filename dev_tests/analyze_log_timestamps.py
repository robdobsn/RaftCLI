import re
from datetime import datetime
import statistics

def parse_log_line(line):
    """
    Parse a single log line to extract the timestamp and message.
    Returns (timestamp, message) if found, otherwise (None, line).
    """
    match = re.search(r"\((\d+) (\d+:\d+:\d+\.\d+)\)", line)
    if match:
        esp32_time = int(match.group(1))  # ESP32 timestamp in milliseconds
        full_timestamp = match.group(2)  # Full PC timestamp
        return (esp32_time, full_timestamp, line)
    return (None, None, line)

def time_difference_in_microseconds(t1, t2):
    """
    Calculate the difference between two datetime timestamps in microseconds.
    """
    dt1 = datetime.strptime(t1, "%H:%M:%S.%f")
    dt2 = datetime.strptime(t2, "%H:%M:%S.%f")
    delta = dt2 - dt1
    return delta.total_seconds() * 1_000_000  # Convert seconds to microseconds

def analyze_log(file_path):
    """
    Analyze the log file for timestamp differences and calculate variance and std dev.
    """
    sections = []
    differences = []
    previous_esp32_time = None
    previous_full_time = None

    with open(file_path, "r") as file:
        for line in file:
            esp32_time, full_timestamp, _ = parse_log_line(line)

            # Only process lines with valid timestamps
            if esp32_time is not None and full_timestamp is not None:
                if previous_esp32_time is not None and previous_full_time is not None:
                    # Calculate ESP32 timestamp difference and full timestamp difference
                    esp32_diff = esp32_time - previous_esp32_time
                    full_time_diff = time_difference_in_microseconds(previous_full_time, full_timestamp)

                    if full_time_diff < 0:
                        print(f"Negative time difference detected: {full_time_diff} µs, skipping.")
                        continue

                    print(f"ESP32 Time: {esp32_time}, Full Time: {full_timestamp}, prev ESP32: {previous_esp32_time}, prev Full: {previous_full_time}, ESP32 Diff: {esp32_diff}, Full Diff: {full_time_diff}")

                    # Check for ESP32 reset
                    if esp32_diff < 0:
                        if len(differences) > 10:
                            sections.append(differences.copy())
                        print("ESP32 timestamp reset detected. Resetting calculations.")
                        differences.clear()
                    else:
                        differences.append(full_time_diff - (esp32_diff * 1_000))

                previous_esp32_time = esp32_time
                previous_full_time = full_timestamp

    # Calculate statistics
    for section in sections:
        variance = statistics.variance(section)
        std_dev = statistics.stdev(section)
        print(f"Section Variance: {variance:.2f} µs^2, Standard Deviation: {std_dev:.2f} µs, Count: {len(section)}")

# Run the analysis
log_file_path = "log_with_timestamps.log"  # Replace with your actual log file path
analyze_log(log_file_path)
