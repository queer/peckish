# artifact

```yaml
name: "my deb artifact"
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
package_name: "my-cool-package"
package_maintainer: "my name goes here <me@example.com>"
package_version: "1.0.0-1"
package_arch: "amd64"
package_depends: ""
package_description: "my cool deb package"
```
