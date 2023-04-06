# peckish

peckish (case-sensitive) is a tool for repackaging Linux software artifacts.

For example, suppose you're an application developer. You just made something
cool and want to distribute it. However, packaging is *hard*. Different package
formats do things differently -- ex. Arch has `x86_64` and `any` as architectures,
but Debian has over a dozen and calls x86_64 `amd64` -- and it's hard to
remember all the specifics. This is compounded by having to figure out the
appropriate CLI flags for each package format. How many people can write a
valid `tar` command on the first try? :P

## usage

Create a `peckish.yaml` file in the root of your project. Documentation of
specific artifact types can be found in the `docs/` directory.

```yaml
# whether to run as a pipeline, ie each artifact output is the input to the
# next producer
pipeline: false

metadata:
  name: "whatever"
  version: "0.1.0-1"
  description: "a package"
  author: "me"
  arch: "amd64"

# the artifact being used as input to the pipeline.
input:
  name: "some file"
  type: "file"
  paths:
  - "./path/to/file"

# the producers being used as pipeline outputs.
output:
  - name: "tarball"
    type: "tarball"
    path: "./whatever.tar"

  - name: "debian package"
    type: "deb"
    path: "./whatever.deb"
```

### library

```rust
// pipelines
use peckish::prelude::pipeline::*;
use peckish::prelude::*;

let file_artifact = ...;

let tarball_producer = ...;

let debian_producer = ...;

let config = PeckishConfig {
    input: ConfiguredArtifact::File(file_artifact),
    output: vec![
        ConfiguredProducer::Tarball(tarball_producer),
        ConfiguredProducer::Deb(debian_producer),
    ],
    pipeline: false,
};

let pipeline = Pipeline::new();
let out = pipeline.run(config).await?;
println!("produced {} artifacts", out.len());

// artifacts
use peckish::prelude::builder::*;
use peckish::prelude::*;

let file_artifact = FileArtifactBuilder::new("example file artifact".into())
    .add_path("./examples/a".into())
    .build()?;

let tarball_producer = TarballProducerBuilder::new("example tarball producer".into())
    .path("test.tar.gz".into())
    .build()?;

let tarball_artifact = tarball_producer.produce(&file_artifact).await?;
```

# license

Copyright 2023-present amy null

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
