use anyhow::{Context, Result};
use clap::Parser;
use plotters::prelude::*;
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "memory_tracker")]
#[command(about = "Track memory usage of a process and generate statistics")]
struct Cli {
    /// Process ID to monitor
    #[arg(short, long)]
    pid: u32,

    /// Sampling interval in milliseconds
    #[arg(short, long, default_value = "1000")]
    interval: u64,

    /// Output image file path
    #[arg(short, long, default_value = "memory_usage.png")]
    output: String,

    /// Duration to monitor in seconds (0 = until process exits)
    #[arg(short, long, default_value = "0")]
    duration: u64,

    /// Optional file path to save memory data as CSV (time,memory_kb)
    #[arg(short = 'c', long)]
    csv_output: Option<String>,
}

#[derive(Debug)]
struct MemoryStats {
    samples: Vec<(f64, u64)>, // (time_seconds, memory_kb)
}

impl MemoryStats {
    fn new() -> Self {
        Self {
            samples: Vec::new(),
        }
    }

    fn add_sample(&mut self, time: f64, memory_kb: u64) {
        self.samples.push((time, memory_kb));
    }

    fn mean(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.samples.iter().map(|(_, mem)| mem).sum();
        sum as f64 / self.samples.len() as f64
    }

    fn median(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let mut values: Vec<u64> = self.samples.iter().map(|(_, mem)| *mem).collect();
        values.sort_unstable();
        let mid = values.len() / 2;
        if values.len() % 2 == 0 {
            (values[mid - 1] + values[mid]) as f64 / 2.0
        } else {
            values[mid] as f64
        }
    }

    fn max(&self) -> u64 {
        self.samples.iter().map(|(_, mem)| mem).max().copied().unwrap_or(0)
    }

    fn min(&self) -> u64 {
        self.samples.iter().map(|(_, mem)| mem).min().copied().unwrap_or(0)
    }
}

fn read_memory_usage(pid: u32) -> Result<u64> {
    let status_path = format!("/proc/{}/status", pid);
    let content = fs::read_to_string(&status_path)
        .with_context(|| format!("Failed to read {}", status_path))?;

    for line in content.lines() {
        if line.starts_with("VmRSS:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let memory_kb = parts[1]
                    .parse::<u64>()
                    .with_context(|| format!("Failed to parse memory value: {}", parts[1]))?;
                return Ok(memory_kb);
            }
        }
    }

    anyhow::bail!("VmRSS not found in /proc/{}/status", pid);
}

fn generate_chart(stats: &MemoryStats, output_path: &str) -> Result<()> {
    let root = BitMapBackend::new(output_path, (1024, 768)).into_drawing_area();
    root.fill(&WHITE)?;

    let max_time = stats.samples.last().map(|(t, _)| *t).unwrap_or(0.0);
    let max_memory_mb = stats.max() as f64 / 1024.0;
    let min_memory_mb = stats.min() as f64 / 1024.0;

    let y_margin = (max_memory_mb - min_memory_mb) / 10.0;
    let y_min = (min_memory_mb - y_margin).max(0.0);
    let y_max = max_memory_mb + y_margin;

    let mut chart = ChartBuilder::on(&root)
        .caption("Memory Usage Over Time", ("sans-serif", 40))
        .margin(10)
        .x_label_area_size(40)
        .y_label_area_size(60)
        .build_cartesian_2d(0f64..max_time, y_min..y_max)?;

    chart
        .configure_mesh()
        .x_desc("Time (seconds)")
        .y_desc("Memory (MB)")
        .draw()?;

    chart.draw_series(LineSeries::new(
        stats.samples.iter().map(|(t, m)| (*t, *m as f64 / 1024.0)),
        &BLUE,
    ))?;

    root.present()?;
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    println!("Monitoring process {} with interval {}ms", cli.pid, cli.interval);
    if cli.duration > 0 {
        println!("Duration: {} seconds", cli.duration);
    } else {
        println!("Duration: until process exits");
    }

    let mut stats = MemoryStats::new();
    let start_time = Instant::now();
    let interval = Duration::from_millis(cli.interval);
    let max_duration = if cli.duration > 0 {
        Some(Duration::from_secs(cli.duration))
    } else {
        None
    };

    loop {
        let elapsed = start_time.elapsed();

        if let Some(max_dur) = max_duration {
            if elapsed >= max_dur {
                println!("\nReached maximum duration");
                break;
            }
        }

        match read_memory_usage(cli.pid) {
            Ok(memory_kb) => {
                let time_secs = elapsed.as_secs_f64();
                stats.add_sample(time_secs, memory_kb);
                print!("\rTime: {:.1}s | Memory: {} KB ({:.2} MB)",
                       time_secs, memory_kb, memory_kb as f64 / 1024.0);
                std::io::Write::flush(&mut std::io::stdout())?;
            }
            Err(e) => {
                println!("\nProcess {} no longer exists or is not accessible: {}", cli.pid, e);
                break;
            }
        }

        thread::sleep(interval);
    }

    println!("\n\nGenerating statistics...");
    println!("Total samples: {}", stats.samples.len());
    println!("Mean memory: {:.2} KB ({:.2} MB)", stats.mean(), stats.mean() / 1024.0);
    println!("Median memory: {:.2} KB ({:.2} MB)", stats.median(), stats.median() / 1024.0);
    println!("Max memory: {} KB ({:.2} MB)", stats.max(), stats.max() as f64 / 1024.0);
    println!("Min memory: {} KB ({:.2} MB)", stats.min(), stats.min() as f64 / 1024.0);

    if !stats.samples.is_empty() {
        println!("\nGenerating chart: {}", cli.output);
        generate_chart(&stats, &cli.output)?;
        println!("Chart saved successfully!");
    } else {
        println!("\nNo samples collected, skipping chart generation");
    }

    // Save CSV if requested
    if let Some(csv_path) = &cli.csv_output {
        println!("\nSaving memory data to CSV: {}", csv_path);
        let mut csv_content = String::new();
        for (_, memory) in &stats.samples {
            csv_content.push_str(&format!("{}\n", memory));
        }
        fs::write(csv_path, csv_content)
            .with_context(|| format!("Failed to write CSV file: {}", csv_path))?;
        println!("CSV saved successfully!");
    }

    Ok(())
}
