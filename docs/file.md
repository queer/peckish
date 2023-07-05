# artifact

```yaml
name: "my file artifact"
type: "file"
# Paths to include in the artifact. May be files or directories. If a
# directory, contents will be recursively added.
paths:
- "./path/to/include"
- "/other/path/to/include"
```

# producer

```yaml
name: "my file artifact producer"
# The path to the **directory** in which output files will be placed.
path: "/path/to/output"
# Whether or not to preserve empty directories. If false, only directories that
# contain files will be present in the artifact. Defaults to false.
preserve_empty_directories: true # optional
```
