mod prelude {
    pub use ::tap::prelude::*;

    pub use ::anyhow::Context as _;
    pub use ::euclid::default as euclid;
}

fn main() {
    println!("Hello, world!");
}
