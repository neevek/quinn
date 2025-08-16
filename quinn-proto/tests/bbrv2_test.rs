use quinn_proto::congestion::{bbrv2::BbrV2Config, Controller, ControllerFactory};
use quinn_proto::Instant;
use std::sync::Arc;

#[test]
fn test_bbrv2_basic_functionality() {
    let config = Arc::new(BbrV2Config::default());
    let now = Instant::now();
    let mut controller = config.build(now, 1500);
    
    // Test initial window
    assert!(controller.initial_window() > 0);
    assert!(controller.window() > 0);
    
    // Test cloning
    let cloned = controller.clone_box();
    assert_eq!(controller.initial_window(), cloned.initial_window());
    
    // Test metrics
    let metrics = controller.metrics();
    assert!(metrics.congestion_window > 0);
}

#[test]
fn test_bbrv2_config() {
    let mut config = BbrV2Config::default();
    config.initial_window(10000);
    
    let config_arc = Arc::new(config);
    let now = Instant::now();
    let controller = config_arc.build(now, 1500);
    
    assert_eq!(controller.initial_window(), 10000);
}