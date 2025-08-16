#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use crate::{congestion::{bbrv2::BbrV2Config, Controller, ControllerFactory}, Instant};
    
    #[test]
    fn test_bbrv2_creation() {
        let config = Arc::new(BbrV2Config::default());
        let now = Instant::now();
        let controller = config.build(now, 1500);
        
        // Basic sanity checks
        assert!(controller.initial_window() > 0);
        assert!(controller.window() > 0);
    }
    
    #[test]
    fn test_bbrv2_clone() {
        let config = Arc::new(BbrV2Config::default());
        let now = Instant::now();
        let controller = config.build(now, 1500);
        let cloned = controller.clone_box();
        
        assert_eq!(controller.initial_window(), cloned.initial_window());
        assert_eq!(controller.window(), cloned.window());
    }
}