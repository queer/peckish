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
    pub use crate::artifact::{Artifact, ArtifactProducer, SelfBuilder, SelfValidation};
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

    pub mod artifact {
        pub use crate::artifact::arch::ArchArtifact;
        pub use crate::artifact::deb::DebArtifact;
        pub use crate::artifact::docker::DockerArtifact;
        pub use crate::artifact::file::FileArtifact;
        pub use crate::artifact::get_artifact_size;
        pub use crate::artifact::rpm::RpmArtifact;
        pub use crate::artifact::tarball::TarballArtifact;
    }

    pub mod producer {
        pub use crate::artifact::arch::ArchProducer;
        pub use crate::artifact::deb::DebProducer;
        pub use crate::artifact::docker::DockerProducer;
        pub use crate::artifact::file::FileProducer;
        pub use crate::artifact::rpm::RpmProducer;
        pub use crate::artifact::tarball::TarballProducer;
    }

    pub mod builder {
        pub use crate::artifact::arch::{ArchArtifactBuilder, ArchProducerBuilder};
        pub use crate::artifact::deb::{DebArtifactBuilder, DebProducerBuilder};
        pub use crate::artifact::docker::{DockerArtifactBuilder, DockerProducerBuilder};
        pub use crate::artifact::file::{FileArtifactBuilder, FileProducerBuilder};
        pub use crate::artifact::rpm::{RpmArtifactBuilder, RpmProducerBuilder};
        pub use crate::artifact::tarball::{TarballArtifactBuilder, TarballProducerBuilder};
        pub use crate::artifact::SelfBuilder;
    }

    pub mod pipeline {
        pub use crate::pipeline::Pipeline;
        pub use crate::util::config::{ConfiguredArtifact, ConfiguredProducer, PeckishConfig};
    }
}
