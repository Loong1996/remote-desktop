use rd_agent::video::openh264_encoder::Openh264Encoder;
use rd_agent::video::pipeline::VideoPipeline;
use rd_agent::video::testpattern::TestPatternSource;
use rd_agent::video::{EncodedSample, SampleSink};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct RecordingSink(Mutex<Vec<EncodedSample>>);
impl SampleSink for RecordingSink {
    fn write(&self, sample: EncodedSample) {
        self.0.lock().unwrap().push(sample);
    }
}

#[test]
fn testpattern_pipeline_produces_encoded_keyframe() {
    let sink = Arc::new(RecordingSink::default());
    let capturer = Box::new(TestPatternSource { width: 128, height: 72, fps: 30 });
    let encoder = Box::new(Openh264Encoder::new(64, 64, 1_000_000, 30.0).unwrap());
    let pipeline = VideoPipeline::start(capturer, encoder, sink.clone(), 64, 64, 60);
    std::thread::sleep(std::time::Duration::from_millis(500));
    drop(pipeline); // stop
    let samples = sink.0.lock().unwrap();
    assert!(!samples.is_empty(), "pipeline produced no samples");
    assert!(samples[0].keyframe, "first sample should be a forced keyframe");
    assert!(!samples[0].data.is_empty());
}
