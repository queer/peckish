# artifact

```yaml
name: "my arch artifact"
path: "./path-to-artifact.pkg.tar"
```

# producer

For information about specific package metadata, see:

- https://wiki.archlinux.org/title/Arch_package_guidelines
- https://wiki.archlinux.org/title/creating_packages

```yaml
name: "my arch artifact producer"
path: "./path-to-output-artifact.pkg.tar"
# package metadata
package_name: "my-cool-package"
package_ver: "1.0.0-1"
package_desc: "my cool arch package"
package_author: "my name goes here <me@example.com>"
package_arch: "x86_64"
```
