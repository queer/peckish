# artifact

```yaml
name: "my file artifact"
# Paths to include in the artifact. May be files or directories. If a
# directory, contents will be recursively added.
paths:
- "./path/to/include"
- "/other/path/to/include"
# Whether or not to treat paths as prefixes. For example, if the given path is
# "/foo/bar", every path under it will have `/foo/bar` stripped from it before
# it is added to the artifact. Defaults to false.
strip_path_prefixes: true # optional
```

# producer

```yaml
name: "my file artifact producer"
path: "/path/to/output"
# Whether or not to preserve empty directories. If false, only directories that
# contain files will be present in the artifact. Defaults to false.
preserve_empty_directories: true # optional
```
