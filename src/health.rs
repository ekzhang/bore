//! Connection health monitoring and metrics collection.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::time::interval;
use tracing::{info, warn};

/// Connection health metrics
#[derive(Debug, Clone)]
pub struct ConnectionMetrics {
    /// Total connection attempts
    pub total_attempts: u64,
    /// Successful connections
    pub successful_connections: u64,
    /// Failed connections
    pub failed_connections: u64,
    /// Current uptime duration
    pub current_uptime: Duration,
    /// Average latency for recent connections (ms)
    pub avg_latency_ms: f64,
    /// Connection stability score (0.0-1.0)
    pub stability_score: f64,
    /// Bytes transferred in current session
    pub bytes_transferred: u64,
}

/// Real-time connection health monitor
#[derive(Debug)]
pub struct HealthMonitor {
    /// Total connection attempts
    total_attempts: AtomicU64,
    /// Successful connections
    successful_connections: AtomicU64,
    /// Failed connections
    failed_connections: AtomicU64,
    /// Bytes transferred
    bytes_transferred: AtomicU64,
    /// Recent latency measurements (last 100)
    recent_latencies: Arc<Mutex<VecDeque<Duration>>>,
    /// Connection events history
    connection_events: Arc<Mutex<VecDeque<ConnectionEvent>>>,
    /// Session start time
    session_start: Instant,
    /// Last successful connection time
    last_success: Arc<Mutex<Option<Instant>>>,
}

#[derive(Debug, Clone)]
/// Connection event for health monitoring
pub struct ConnectionEvent {
    /// When the event occurred
    pub timestamp: Instant,
    /// Type of connection event
    pub event_type: EventType,
    /// Optional latency measurement
    pub latency: Option<Duration>,
}

/// Types of connection events tracked by health monitor
#[derive(Debug, Clone, Copy)]
pub enum EventType {
    /// A connection attempt was initiated
    ConnectionAttempt,
    /// A connection was successfully established
    ConnectionSuccess,
    /// A connection attempt failed
    ConnectionFailure,
    /// An existing connection was lost
    ConnectionLost,
    /// A reconnection occurred
    Reconnection,
}

impl HealthMonitor {
    /// Create a new health monitor
    pub fn new() -> Self {
        Self {
            total_attempts: AtomicU64::new(0),
            successful_connections: AtomicU64::new(0),
            failed_connections: AtomicU64::new(0),
            bytes_transferred: AtomicU64::new(0),
            recent_latencies: Arc::new(Mutex::new(VecDeque::new())),
            connection_events: Arc::new(Mutex::new(VecDeque::new())),
            session_start: Instant::now(),
            last_success: Arc::new(Mutex::new(None)),
        }
    }
    
    /// Record a connection attempt
    pub fn record_attempt(&self) {
        self.total_attempts.fetch_add(1, Ordering::Relaxed);
        self.record_event(EventType::ConnectionAttempt, None);
    }
    
    /// Record a successful connection with latency
    pub fn record_success(&self, latency: Duration) {
        self.successful_connections.fetch_add(1, Ordering::Relaxed);
        *self.last_success.lock().unwrap() = Some(Instant::now());
        
        // Store latency
        let mut latencies = self.recent_latencies.lock().unwrap();
        latencies.push_back(latency);
        if latencies.len() > 100 {
            latencies.pop_front();
        }
        
        self.record_event(EventType::ConnectionSuccess, Some(latency));
    }
    
    /// Record a connection failure
    pub fn record_failure(&self) {
        self.failed_connections.fetch_add(1, Ordering::Relaxed);
        self.record_event(EventType::ConnectionFailure, None);
    }
    
    /// Record connection lost
    pub fn record_connection_lost(&self) {
        self.record_event(EventType::ConnectionLost, None);
    }
    
    /// Record reconnection
    pub fn record_reconnection(&self, latency: Duration) {
        self.record_event(EventType::Reconnection, Some(latency));
    }
    
    /// Add bytes transferred
    pub fn add_bytes_transferred(&self, bytes: u64) {
        self.bytes_transferred.fetch_add(bytes, Ordering::Relaxed);
    }
    
    /// Get current metrics
    pub fn get_metrics(&self) -> ConnectionMetrics {
        let total_attempts = self.total_attempts.load(Ordering::Relaxed);
        let successful = self.successful_connections.load(Ordering::Relaxed);
        let failed = self.failed_connections.load(Ordering::Relaxed);
        
        let current_uptime = self.calculate_current_uptime();
        let avg_latency_ms = self.calculate_avg_latency();
        let stability_score = self.calculate_stability_score();
        let bytes_transferred = self.bytes_transferred.load(Ordering::Relaxed);
        
        ConnectionMetrics {
            total_attempts,
            successful_connections: successful,
            failed_connections: failed,
            current_uptime,
            avg_latency_ms,
            stability_score,
            bytes_transferred,
        }
    }
    
    /// Start periodic health reporting
    pub fn start_health_reporting(self: Arc<Self>, report_interval: Duration) {
        let monitor = Arc::clone(&self);
        tokio::spawn(async move {
            let mut interval = interval(report_interval);
            
            loop {
                interval.tick().await;
                let metrics = monitor.get_metrics();
                
                info!(
                    "Connection Health: attempts={}, success={}, failed={}, uptime={:.1}min, latency={:.1}ms, stability={:.2}%",
                    metrics.total_attempts,
                    metrics.successful_connections,
                    metrics.failed_connections,
                    metrics.current_uptime.as_secs_f64() / 60.0,
                    metrics.avg_latency_ms,
                    metrics.stability_score * 100.0
                );
                
                // Warn if health is degraded
                if metrics.stability_score < 0.8 {
                    warn!("Connection stability degraded: {:.1}%", metrics.stability_score * 100.0);
                }
            }
        });
    }
    
    /// Check if connection is healthy
    pub fn is_healthy(&self) -> bool {
        let metrics = self.get_metrics();
        
        // Consider healthy if:
        // 1. Stability score > 80%
        // 2. Recent connection successful (within last 5 minutes)
        // 3. Average latency < 1000ms
        
        let recent_success = self.last_success.lock().unwrap()
            .map(|last| last.elapsed() < Duration::from_secs(300))
            .unwrap_or(false);
            
        metrics.stability_score > 0.8 && 
        recent_success && 
        metrics.avg_latency_ms < 1000.0
    }
    
    /// Record an event
    fn record_event(&self, event_type: EventType, latency: Option<Duration>) {
        let event = ConnectionEvent {
            timestamp: Instant::now(),
            event_type,
            latency,
        };
        
        let mut events = self.connection_events.lock().unwrap();
        events.push_back(event);
        
        // Keep only last 1000 events
        if events.len() > 1000 {
            events.pop_front();
        }
    }
    
    /// Calculate current uptime
    fn calculate_current_uptime(&self) -> Duration {
        if let Some(last_success) = *self.last_success.lock().unwrap() {
            last_success.duration_since(self.session_start)
        } else {
            Duration::ZERO
        }
    }
    
    /// Calculate average latency from recent measurements
    fn calculate_avg_latency(&self) -> f64 {
        let latencies = self.recent_latencies.lock().unwrap();
        if latencies.is_empty() {
            return 0.0;
        }
        
        let sum: Duration = latencies.iter().sum();
        sum.as_millis() as f64 / latencies.len() as f64
    }
    
    /// Calculate stability score based on recent events
    fn calculate_stability_score(&self) -> f64 {
        let events = self.connection_events.lock().unwrap();
        if events.is_empty() {
            return 1.0;
        }
        
        let now = Instant::now();
        let recent_window = Duration::from_secs(300); // Last 5 minutes
        
        let recent_events: Vec<_> = events
            .iter()
            .filter(|event| now.duration_since(event.timestamp) <= recent_window)
            .collect();
            
        if recent_events.is_empty() {
            return 1.0;
        }
        
        let total_events = recent_events.len() as f64;
        let success_events = recent_events
            .iter()
            .filter(|event| matches!(event.event_type, EventType::ConnectionSuccess | EventType::Reconnection))
            .count() as f64;
            
        let failure_events = recent_events
            .iter()
            .filter(|event| matches!(event.event_type, EventType::ConnectionFailure | EventType::ConnectionLost))
            .count() as f64;
        
        if total_events == 0.0 {
            1.0
        } else {
            // Score based on success rate, penalized by failures
            let success_rate = success_events / total_events;
            let failure_penalty = failure_events / total_events * 0.5;
            (success_rate - failure_penalty).max(0.0).min(1.0)
        }
    }
}

impl Default for HealthMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Global health monitor instance
static HEALTH_MONITOR: std::sync::OnceLock<Arc<HealthMonitor>> = std::sync::OnceLock::new();

/// Get the global health monitor instance
pub fn get_health_monitor() -> Arc<HealthMonitor> {
    HEALTH_MONITOR.get_or_init(|| Arc::new(HealthMonitor::new())).clone()
}

/// Initialize health monitoring with periodic reporting
pub fn init_health_monitoring(report_interval: Duration) {
    let monitor = get_health_monitor();
    monitor.start_health_reporting(report_interval);
}
