//! Adaptive backoff strategies for connection retry logic.

use std::time::{Duration, Instant};

/// Types of connection failures for adaptive backoff
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FailureType {
    /// Network unreachable or DNS resolution failed
    NetworkUnreachable,
    /// Connection refused (server not listening)
    ConnectionRefused,
    /// Connection timeout
    Timeout,
    /// Connection reset by peer
    ConnectionReset,
    /// Authentication failure
    AuthenticationFailed,
    /// Server error response
    ServerError,
    /// Unknown/other error
    Unknown,
}

/// Adaptive backoff strategy that adjusts retry delays based on failure patterns
#[derive(Debug, Clone)]
pub struct AdaptiveBackoff {
    /// Current attempt number
    attempt: u32,
    /// Base delay
    base_delay: Duration,
    /// Maximum delay
    max_delay: Duration,
    /// Multiplier for exponential backoff
    multiplier: f64,
    /// Recent failure history (last 10 failures)
    failure_history: Vec<(Instant, FailureType)>,
    /// Consecutive timeout count
    consecutive_timeouts: u32,
    /// Consecutive network errors count
    consecutive_network_errors: u32,
    /// Last successful connection time
    last_success: Option<Instant>,
}

impl AdaptiveBackoff {
    /// Create a new adaptive backoff with default parameters
    pub fn new(base_delay: Duration, max_delay: Duration) -> Self {
        Self {
            attempt: 0,
            base_delay,
            max_delay,
            multiplier: 2.0,
            failure_history: Vec::new(),
            consecutive_timeouts: 0,
            consecutive_network_errors: 0,
            last_success: None,
        }
    }
    
    /// Record a successful connection
    pub fn record_success(&mut self) {
        self.attempt = 0;
        self.consecutive_timeouts = 0;
        self.consecutive_network_errors = 0;
        self.last_success = Some(Instant::now());
        self.failure_history.clear();
    }
    
    /// Record a failure and get the next retry delay
    pub fn record_failure(&mut self, failure_type: FailureType) -> Duration {
        self.attempt += 1;
        let now = Instant::now();
        
        // Update failure counters
        match failure_type {
            FailureType::Timeout => self.consecutive_timeouts += 1,
            FailureType::NetworkUnreachable | FailureType::ConnectionRefused => {
                self.consecutive_network_errors += 1;
            }
            FailureType::ConnectionReset => {
                // Reset might indicate server restart, use shorter delay
                self.consecutive_timeouts = 0;
            }
            _ => {
                self.consecutive_timeouts = 0;
                self.consecutive_network_errors = 0;
            }
        }
        
        // Keep recent failure history (last 10 failures)
        self.failure_history.push((now, failure_type));
        if self.failure_history.len() > 10 {
            self.failure_history.remove(0);
        }
        
        self.calculate_delay(failure_type)
    }
    
    /// Calculate the appropriate delay based on failure patterns
    fn calculate_delay(&self, current_failure: FailureType) -> Duration {
        let mut delay = self.base_delay;
        
        // Apply exponential backoff
        for _ in 0..self.attempt.saturating_sub(1) {
            delay = Duration::from_millis((delay.as_millis() as f64 * self.multiplier) as u64);
            if delay > self.max_delay {
                delay = self.max_delay;
                break;
            }
        }
        
        // Adaptive adjustments based on failure patterns
        delay = self.adjust_for_failure_pattern(delay, current_failure);
        
        // Add jitter (±25%)
        let jitter_range = delay.as_millis() / 4;
        let jitter = Duration::from_millis(fastrand::u64(0..=jitter_range as u64));
        let jitter_sign = if fastrand::bool() { 1 } else { -1 };
        
        if jitter_sign > 0 {
            delay + jitter
        } else {
            delay.saturating_sub(jitter)
        }
    }
    
    /// Adjust delay based on failure patterns
    fn adjust_for_failure_pattern(&self, mut delay: Duration, current_failure: FailureType) -> Duration {
        match current_failure {
            FailureType::NetworkUnreachable => {
                // Network issues might need longer delays
                if self.consecutive_network_errors >= 3 {
                    delay = delay * 2;
                }
            }
            
            FailureType::ConnectionRefused => {
                // Server might be starting up, use moderate delay
                if self.consecutive_network_errors >= 5 {
                    delay = delay * 3; // Back off more aggressively
                }
            }
            
            FailureType::Timeout => {
                // Frequent timeouts suggest network congestion
                if self.consecutive_timeouts >= 3 {
                    delay = delay * 2;
                }
            }
            
            FailureType::ConnectionReset => {
                // Server restart scenario - use shorter delay initially
                if self.attempt <= 2 {
                    delay = delay / 2;
                }
            }
            
            FailureType::AuthenticationFailed => {
                // Auth failures shouldn't retry too quickly
                delay = std::cmp::max(delay, Duration::from_secs(5));
            }
            
            FailureType::ServerError => {
                // Server errors might indicate overload
                delay = delay * 3;
            }
            
            FailureType::Unknown => {
                // Conservative approach for unknown errors
                delay = Duration::from_millis((delay.as_millis() as f64 * 1.5) as u64);
            }
        }
        
        // Check for rapid consecutive failures (circuit breaker pattern)
        if self.is_rapid_failure_pattern() {
            delay = std::cmp::max(delay, Duration::from_secs(30));
        }
        
        std::cmp::min(delay, self.max_delay)
    }
    
    /// Detect if we're in a rapid failure pattern
    fn is_rapid_failure_pattern(&self) -> bool {
        if self.failure_history.len() < 5 {
            return false;
        }
        
        let now = Instant::now();
        let recent_failures = self.failure_history
            .iter()
            .filter(|(time, _)| now.duration_since(*time) < Duration::from_secs(60))
            .count();
            
        recent_failures >= 5
    }
    
    /// Get current attempt number
    pub fn attempt_count(&self) -> u32 {
        self.attempt
    }
    
    /// Check if we should give up (circuit breaker)
    pub fn should_circuit_break(&self, max_attempts: u32) -> bool {
        if self.attempt >= max_attempts {
            return true;
        }
        
        // Circuit break if we've had too many rapid failures
        if self.is_rapid_failure_pattern() && self.attempt >= max_attempts / 2 {
            return true;
        }
        
        // Circuit break if we've been failing for a very long time
        if let Some(last_success) = self.last_success {
            if last_success.elapsed() > Duration::from_secs(300) && self.attempt >= 10 {
                return true;
            }
        }
        
        false
    }
}

/// Determine failure type from error
pub fn categorize_error(error: &anyhow::Error) -> FailureType {
    let error_string = error.to_string().to_lowercase();
    
    if error_string.contains("timeout") || error_string.contains("timed out") {
        FailureType::Timeout
    } else if error_string.contains("connection refused") {
        FailureType::ConnectionRefused
    } else if error_string.contains("network unreachable") || error_string.contains("no route to host") {
        FailureType::NetworkUnreachable
    } else if error_string.contains("connection reset") || error_string.contains("broken pipe") {
        FailureType::ConnectionReset
    } else if error_string.contains("authentication") || error_string.contains("auth") {
        FailureType::AuthenticationFailed
    } else if error_string.contains("server error") {
        FailureType::ServerError
    } else {
        FailureType::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_adaptive_backoff_basic() {
        let mut backoff = AdaptiveBackoff::new(
            Duration::from_millis(100),
            Duration::from_secs(30)
        );
        
        // First failure should be close to base delay
        let delay1 = backoff.record_failure(FailureType::Timeout);
        assert!(delay1 >= Duration::from_millis(75) && delay1 <= Duration::from_millis(125));
        
        // Second failure should be roughly doubled
        let delay2 = backoff.record_failure(FailureType::Timeout);
        assert!(delay2 > delay1);
        
        // Success should reset
        backoff.record_success();
        let delay3 = backoff.record_failure(FailureType::Timeout);
        assert!(delay3 < delay2);
    }
    
    #[test]
    fn test_failure_type_categorization() {
        assert_eq!(categorize_error(&anyhow::anyhow!("connection timeout")), FailureType::Timeout);
        assert_eq!(categorize_error(&anyhow::anyhow!("Connection refused")), FailureType::ConnectionRefused);
        assert_eq!(categorize_error(&anyhow::anyhow!("Network unreachable")), FailureType::NetworkUnreachable);
    }
}
