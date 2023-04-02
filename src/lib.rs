//! peckish is a tool for converting between different software artifact
//! formats.
//!
//! ## how?
//!
//! The core abstraction is an in-memory filesystem. peckish takes in a given
//! artifact as input, converts it to an in-memory filesystem, manipulates it,
//! and then exports it as another artifact. This is done with the [`Artifact`]
//! and [`ArtifactProducer`] traits. The [`Pipeline`] struct is used to
//! construct a chain of artifacts to convert from one to another.
//!
//! ## supported formats
//!
//! - Arch packages
//! - Debian packages
//! - Docker images
//! - Normal files
//! - Tarballs

pub mod artifact;
mod fs;
pub mod pipeline;
pub mod util;

pub mod prelude {
    pub use crate::artifact::{Artifact, ArtifactProducer};
    pub use crate::pipeline::Pipeline;
    pub use crate::util::config::{Injection, PeckishConfig};

    pub mod arch {
        pub use crate::artifact::arch::*;
    }

    pub mod deb {
        pub use crate::artifact::deb::*;
    }

    pub mod docker {
        pub use crate::artifact::docker::*;
    }

    pub mod file {
        pub use crate::artifact::file::*;
    }

    pub mod tarball {
        pub use crate::artifact::tarball::*;
    }
}
