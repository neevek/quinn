use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;

use rand::{Rng, SeedableRng};

use crate::congestion::ControllerMetrics;
use crate::congestion::bbrv2::bw_estimation::BandwidthEstimation;
use crate::connection::RttEstimator;
use crate::{Duration, Instant};

use super::{BASE_DATAGRAM_SIZE, Controller, ControllerFactory};

mod bw_estimation;
mod min_max;

#[cfg(test)]
mod test;

/// BBRv2 (Bottleneck Bandwidth and Round-trip propagation time v2)
///
/// An improved congestion control algorithm that provides better performance
/// over high bandwidth-delay product networks and handles various network
/// conditions more effectively than BBRv1.
///
/// Based on the IETF draft specification and Google's implementation.
#[derive(Debug, Clone)]
pub struct BbrV2 {
    config: Arc<BbrV2Config>,
    current_mtu: u64,
    
    // Core state
    max_bandwidth: BandwidthEstimation,
    min_rtt: Duration,
    bdp: u64,
    
    // BBRv2 specific state
    inflight_hi: u64,  // Upper bound on inflight data
    inflight_lo: u64,  // Lower bound on inflight data
    #[allow(dead_code)]
    bw_hi: u64,        // Upper bound on bandwidth
    #[allow(dead_code)]
    bw_lo: u64,        // Lower bound on bandwidth
    #[allow(dead_code)]
    ecn_alpha: f64,    // ECN-based congestion signal
    
    // State machine
    mode: Mode,
    cycle_count: u64,
    last_cycle_start: Option<Instant>,
    current_cycle_offset: u8,
    
    // Gains and parameters
    pacing_gain: f32,
    cwnd_gain: f32,
    
    // Tracking variables
    acked_bytes: u64,
    lost_bytes: u64,
    sent_bytes: u64,
    delivered_bytes: u64,
    
    // Packet tracking
    max_acked_packet_number: u64,
    max_sent_packet_number: u64,
    current_round_trip_end_packet_number: u64,
    round_count: u64,
    
    // Timing
    last_ack_time: Option<Instant>,
    last_probe_rtt: Option<Instant>,
    exit_probe_rtt_at: Option<Instant>,
    
    // Window management
    cwnd: u64,
    pacing_rate: u64,
    recovery_window: u64,
    
    // Adaptive parameters
    full_pipe: bool,
    round_start: bool,
    
    random_number_generator: rand::rngs::StdRng,
}

impl BbrV2 {
    /// Construct a new BBRv2 controller
    pub fn new(config: Arc<BbrV2Config>, current_mtu: u16) -> Self {
        let initial_window = config.initial_window;
        let _min_cwnd = calculate_min_window(current_mtu as u64);
        
        Self {
            config,
            current_mtu: current_mtu as u64,
            max_bandwidth: BandwidthEstimation::default(),
            min_rtt: Duration::from_millis(1), // Initialize with small value
            bdp: 0,
            
            // BBRv2 specific state
            inflight_hi: u64::MAX,
            inflight_lo: 0,
            bw_hi: u64::MAX,
            bw_lo: 0,
            ecn_alpha: 0.0,
            
            // State machine
            mode: Mode::Startup,
            cycle_count: 0,
            last_cycle_start: None,
            current_cycle_offset: 0,
            
            // Gains and parameters
            pacing_gain: K_HIGH_GAIN,
            cwnd_gain: K_HIGH_GAIN,
            
            // Tracking variables
            acked_bytes: 0,
            lost_bytes: 0,
            sent_bytes: 0,
            delivered_bytes: 0,
            
            // Packet tracking
            max_acked_packet_number: 0,
            max_sent_packet_number: 0,
            current_round_trip_end_packet_number: 0,
            round_count: 0,
            
            // Timing
            last_ack_time: None,
            last_probe_rtt: None,
            exit_probe_rtt_at: None,
            
            // Window management
            cwnd: initial_window,
            pacing_rate: 0,
            recovery_window: 0,
            
            // Adaptive parameters
            full_pipe: false,
            round_start: false,
            
            random_number_generator: rand::rngs::StdRng::from_os_rng(),
        }
    }
    
    fn enter_startup_mode(&mut self) {
        self.mode = Mode::Startup;
        self.pacing_gain = K_HIGH_GAIN;
        self.cwnd_gain = K_HIGH_GAIN;
    }
    
    fn enter_drain_mode(&mut self) {
        self.mode = Mode::Drain;
        self.pacing_gain = 1.0 / K_HIGH_GAIN;
        self.cwnd_gain = K_HIGH_GAIN;
    }
    
    fn enter_probe_bw_mode(&mut self, now: Instant) {
        self.mode = Mode::ProbeBw;
        self.cwnd_gain = K_DERIVED_HIGH_CWNDGAIN;
        self.last_cycle_start = Some(now);
        
        // Pick a random offset for the gain cycle
        let mut rand_index = self
            .random_number_generator
            .random_range(0..K_PACING_GAIN.len() as u8 - 1);
        if rand_index >= 1 {
            rand_index += 1;
        }
        self.current_cycle_offset = rand_index;
        self.pacing_gain = K_PACING_GAIN[rand_index as usize];
    }
    
    fn enter_probe_rtt_mode(&mut self, now: Instant) {
        self.mode = Mode::ProbeRtt;
        self.pacing_gain = 1.0;
        self.exit_probe_rtt_at = None;
        self.last_probe_rtt = Some(now);
    }
    
    fn update_bdp(&mut self) {
        let bw = self.max_bandwidth.get_estimate();
        if bw > 0 && !self.min_rtt.is_zero() {
            self.bdp = (bw as u128 * self.min_rtt.as_micros() as u128 / 1_000_000) as u64;
        }
    }
    
    fn update_pacing_rate(&mut self) {
        let bw = self.max_bandwidth.get_estimate();
        if bw == 0 {
            return;
        }
        
        self.pacing_rate = ((bw as f64 * self.pacing_gain as f64) as u64)
            .max(self.current_mtu * 100); // Minimum reasonable pacing rate
    }
    
    fn update_congestion_window(&mut self, bytes_acked: u64) {
        self.update_bdp();
        
        let target_cwnd = if self.bdp > 0 {
            ((self.bdp as f64 * self.cwnd_gain as f64) as u64)
                .max(self.config.initial_window)
                .max(calculate_min_window(self.current_mtu))
        } else {
            self.config.initial_window
        };
        
        // Apply inflight bounds
        let bounded_cwnd = target_cwnd.min(self.inflight_hi).max(self.inflight_lo);
        
        match self.mode {
            Mode::Startup => {
                if !self.full_pipe {
                    self.cwnd += bytes_acked;
                } else {
                    self.cwnd = bounded_cwnd.min(self.cwnd + bytes_acked);
                }
            }
            Mode::Drain => {
                self.cwnd = bounded_cwnd;
            }
            Mode::ProbeBw | Mode::ProbeRtt => {
                self.cwnd = bounded_cwnd.min(self.cwnd + bytes_acked);
            }
        }
        
        // Ensure minimum window
        self.cwnd = self.cwnd.max(calculate_min_window(self.current_mtu));
    }
    
    fn check_full_pipe(&mut self) {
        if self.full_pipe {
            return;
        }
        
        // Check if we've filled the pipe
        if self.bdp > 0 {
            let threshold = (self.bdp as f64 * 0.8) as u64; // 80% of BDP
            if self.delivered_bytes > threshold {
                self.full_pipe = true;
            }
        }
    }
    
    fn update_cycle_phase(&mut self, now: Instant, _in_flight: u64) {
        if self.mode != Mode::ProbeBw {
            return;
        }
        
        // Advance cycle after RTT or when conditions are met
        let should_advance = self
            .last_cycle_start
            .map(|last| now.duration_since(last) > self.min_rtt)
            .unwrap_or(false);
            
        if should_advance {
            self.current_cycle_offset = (self.current_cycle_offset + 1) % K_PACING_GAIN.len() as u8;
            self.last_cycle_start = Some(now);
            self.pacing_gain = K_PACING_GAIN[self.current_cycle_offset as usize];
            self.cycle_count += 1;
        }
    }
    
    fn maybe_probe_rtt(&mut self, now: Instant) {
        let should_probe = self.last_probe_rtt
            .map(|last| now.saturating_duration_since(last) > Duration::from_secs(10))
            .unwrap_or(true);
            
        if should_probe && self.mode != Mode::ProbeRtt {
            self.enter_probe_rtt_mode(now);
        }
    }
    
    fn update_recovery_window(&mut self, _bytes_acked: u64, bytes_lost: u64) {
        if self.recovery_window == 0 {
            self.recovery_window = self.cwnd.max(calculate_min_window(self.current_mtu));
        }
        
        if bytes_lost > 0 {
            self.recovery_window = self.recovery_window.saturating_sub(bytes_lost);
        }
        
        self.recovery_window = self.recovery_window
            .max(self.cwnd)
            .max(calculate_min_window(self.current_mtu));
    }
    
    #[allow(dead_code)]
    fn handle_ecn(&mut self, ce_count: u64, total_packets: u64) {
        if total_packets > 0 {
            let alpha = ce_count as f64 / total_packets as f64;
            // Exponentially weighted moving average
            self.ecn_alpha = 0.9 * self.ecn_alpha + 0.1 * alpha;
        }
    }
}

impl Controller for BbrV2 {
    fn on_sent(&mut self, now: Instant, bytes: u64, last_packet_number: u64) {
        self.max_sent_packet_number = last_packet_number;
        self.sent_bytes += bytes;
        self.max_bandwidth.on_sent(now, bytes);
    }
    
    fn on_ack(
        &mut self,
        now: Instant,
        sent: Instant,
        bytes: u64,
        app_limited: bool,
        rtt: &RttEstimator,
    ) {
        self.acked_bytes += bytes;
        self.delivered_bytes += bytes;
        self.last_ack_time = Some(now);
        
        self.max_bandwidth
            .on_ack(now, sent, bytes, self.round_count, app_limited);
            
        // Update RTT estimate
        if self.min_rtt > rtt.min() || self.min_rtt.is_zero() {
            self.min_rtt = rtt.min();
        }
    }
    
    fn on_end_acks(
        &mut self,
        now: Instant,
        in_flight: u64,
        app_limited: bool,
        largest_packet_num_acked: Option<u64>,
    ) {
        if let Some(largest_acked) = largest_packet_num_acked {
            self.max_acked_packet_number = largest_acked;
        }
        
        let bytes_acked = self.max_bandwidth.bytes_acked_this_window();
        self.max_bandwidth.end_acks(self.round_count, app_limited);
        
        // Detect round start
        self.round_start = self.max_acked_packet_number > self.current_round_trip_end_packet_number;
        if self.round_start {
            self.current_round_trip_end_packet_number = self.max_sent_packet_number;
            self.round_count += 1;
        }
        
        // State machine transitions
        match self.mode {
            Mode::Startup => {
                self.check_full_pipe();
                if self.full_pipe && self.round_start {
                    self.enter_drain_mode();
                }
            }
            Mode::Drain => {
                if in_flight <= self.bdp {
                    self.enter_probe_bw_mode(now);
                }
            }
            Mode::ProbeBw => {
                self.update_cycle_phase(now, in_flight);
            }
            Mode::ProbeRtt => {
                if self.exit_probe_rtt_at.is_none() {
                    if in_flight < calculate_min_window(self.current_mtu) + self.current_mtu {
                        self.exit_probe_rtt_at = Some(now + Duration::from_millis(200));
                    }
                } else if now >= self.exit_probe_rtt_at.unwrap() {
                    if self.full_pipe {
                        self.enter_probe_bw_mode(now);
                    } else {
                        self.enter_startup_mode();
                    }
                }
            }
        }
        
        // Periodic RTT probing
        self.maybe_probe_rtt(now);
        
        // Update core parameters
        self.update_pacing_rate();
        self.update_congestion_window(bytes_acked);
        
        // Reset for next round
        self.acked_bytes = 0;
        self.lost_bytes = 0;
    }
    
    fn on_congestion_event(
        &mut self,
        _now: Instant,
        _sent: Instant,
        _is_persistent_congestion: bool,
        lost_bytes: u64,
    ) {
        self.lost_bytes += lost_bytes;
        self.update_recovery_window(self.acked_bytes, lost_bytes);
    }
    
    fn on_mtu_update(&mut self, new_mtu: u16) {
        self.current_mtu = new_mtu as u64;
        self.cwnd = self.cwnd.max(calculate_min_window(self.current_mtu));
    }
    
    fn window(&self) -> u64 {
        if self.mode == Mode::ProbeRtt {
            calculate_min_window(self.current_mtu)
        } else if self.recovery_window > 0 {
            self.cwnd.min(self.recovery_window)
        } else {
            self.cwnd
        }
    }
    
    fn metrics(&self) -> ControllerMetrics {
        ControllerMetrics {
            congestion_window: self.window(),
            ssthresh: None,
            pacing_rate: Some(self.pacing_rate * 8), // Convert to bits/sec
        }
    }
    
    fn clone_box(&self) -> Box<dyn Controller> {
        Box::new(self.clone())
    }
    
    fn initial_window(&self) -> u64 {
        self.config.initial_window
    }
    
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

/// Configuration for the [`BbrV2`] congestion controller
#[derive(Debug, Clone)]
pub struct BbrV2Config {
    initial_window: u64,
}

impl BbrV2Config {
    /// Set the initial congestion window
    pub fn initial_window(&mut self, value: u64) -> &mut Self {
        self.initial_window = value;
        self
    }
}

impl Default for BbrV2Config {
    fn default() -> Self {
        Self {
            initial_window: K_MAX_INITIAL_CONGESTION_WINDOW * BASE_DATAGRAM_SIZE,
        }
    }
}

impl ControllerFactory for BbrV2Config {
    fn build(self: Arc<Self>, _now: Instant, current_mtu: u16) -> Box<dyn Controller> {
        Box::new(BbrV2::new(self, current_mtu))
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Mode {
    Startup,
    Drain,
    ProbeBw,
    ProbeRtt,
}

fn calculate_min_window(current_mtu: u64) -> u64 {
    4 * current_mtu
}

// Constants
const K_HIGH_GAIN: f32 = 2.885;
const K_DERIVED_HIGH_CWNDGAIN: f32 = 2.0;
const K_PACING_GAIN: [f32; 8] = [1.25, 0.75, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
const K_MAX_INITIAL_CONGESTION_WINDOW: u64 = 200;