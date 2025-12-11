#![expect(unused)]
#![expect(default_numeric_fallback)]
#![expect(clippy::uninlined_format_args)]

mod app;
mod capture;
mod converter;
mod encoder;
mod stream;
mod encoding_thread;
mod resample;

fn main() {
    pretty_env_logger::init();
    app::run();
}
