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
