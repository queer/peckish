# artifact

```yaml
name: "my docker artifact"
image: "my/image:latest"
```

# producer

```yaml
name: "my docker artifact producer"
image: "output/image:latest"
# package metadata
base_image: "ubuntu:latest" # optional
cmd: ["/bin/sh", "-c", "echo asdf"] # optional
```
