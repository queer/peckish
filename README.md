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
let config = PeckishConfig {
    pipeline: false,
    input: ConfiguredArtifact::File(FileArtifact {
        name: "my files".into(),
        paths: vec!["...".into()],
        strip_path_prefixes: None,
    }),
    output: vec![ConfiguredProducer::Tarball(TarballProducer {
        name: "tarball for whatever".into(),
        path: PathBuf::from("..."), // or otherwise
        injections: vec![],
    })],
};

let pipeline = Pipeline::new(true);
pipeline.run().await?;

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
