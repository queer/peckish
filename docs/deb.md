# artifact

```yaml
name: "my deb artifact"
type: "deb"
path: "./path-to-artifact.deb"
```

# producer

For more information about package-specific metadata, see:

- https://wiki.debian.org/Packaging
- https://www.debian.org/doc/manuals/packaging-tutorial/packaging-tutorial.en.pdf

```yaml
name: "my deb artifact producer"
path: "./path-to-output-artifact.deb"
# package metadata
prerm: "./path-to-prerm-script" # optional
postinst: "./path-to-postinst-script" # optional
depends: "libc6" # optional
```
