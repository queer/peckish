# peckish

peckish is a tool for repackaging software artifacts.

## usage

Create a `peckish.yaml` file in the root of your project.

```yaml
# whether to run as a pipeline, ie each artifact output is the input to the
# next producer
pipeline: bool

# the artifact being used as input to the pipeline. look at the `InputArtifact`
# enum for now.
input:
  name: string
  type: string
  # ...

# the producers being used as pipeline outputs. look at the `OutputProducer`
# enum for now
output:
  - name: "tarball"
    type: "tarball"
    path: "./whatever.tar"

  # ...
```
