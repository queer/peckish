# peckish

peckish is a tool for repackaging software artifacts.

## usage

Create a `peckish.yaml` file in the root of your project. Documentation of
specific artifact types can be found in the `docs/` directory.

```yaml
# whether to run as a pipeline, ie each artifact output is the input to the
# next producer
pipeline: false

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

  # ...
```

### library

```rust
// pipelines
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
