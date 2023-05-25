# injections

Injections are a way to manipulate the filesystem of an artifact that is being
produced, before it's written to the filesystem. For example, suppose you're
making a new Rust project. Your binary is at `target/release/binary`, but when
you take in that artifact, it's loaded into the memfs at
`/target/release/binary`, and that's no good. To fix this, you can use
injections to move it to the correct location:

```yaml
metadata:
  name: "peckish"
  version: "0.0.1-1"
  description: "peckish transforms software artifacts"
  author: "amy"
  arch: "amd64"
  license: "Apache-2.0"

input:
  name: "peckish release binary"
  type: "file"
  paths:
    - "./target/release/peckish"

output:
  - name: "peckish.arch.pkg.tar"
    type: "arch"
    path: "./release/peckish.arch.pkg.tar"
    injections:
      - type: "move"
        src: "/target/release/peckish"
        dest: "/usr/bin/peckish"
```

As you can see, the `arch` package output has an `injections` key. This is a
list of injections that will be applied to the artifact before it's written to
the filesystem. In this case, we're moving the file from
`/target/release/peckish` to `/usr/bin/peckish`.

Note that when doing things like moving files, the parent directories will
remain in the artifact's memfs. Cleaning up empty directories is your
responsibility.

## supported injections

- move `"move"`

  Moves a file or directory from one location to another. The `src` and `dest`
  keys are required.

  ```yaml
  injections:
    - type: "move"
      src: "/target/release/peckish"
      dest: "/usr/bin/peckish"
  ```

- copy `"copy"`

  Copies a file or directory from one location to another. The `src` and `dest`
  keys are required.

  ```yaml
  injections:
    - type: "copy"
      src: "/target/release/peckish"
      dest: "/usr/bin/peckish"
  ```

- symlink `"symlink"`

  Creates a symlink from one location to another. The `src` and `dest` keys are
  required.

  ```yaml
  injections:
    # Creates a symlink at `dest` that points to `src`.
    - type: "symlink"
      src: "/target/release/peckish"
      dest: "/usr/bin/peckish"
  ```

- touch `"touch"`

  Creates an empty file at the specified location. The `path` key is required.

  ```yaml
  injections:
    - type: "touch"
      path: "/usr/bin/peckish"
  ```

- delete `"delete"`

  Deletes a file or directory at the specified location. The `path` key is
  required.

  ```yaml
  injections:
    - type: "delete"
      path: "/usr/bin/peckish"
  ```

- create `"create"`

  Creates a file with the given contents at the specified location. The
  `path` and `content` keys are required.

  ```yaml
  injections:
    - type: "create"
      path: "/hello.txt"
      content: "hello world"
  ```

- host file `"host_file"`

  Copies a file from the host to the given location in the artifact.

  ```yaml
  injections:
    - type: "host_file"
      src: "/etc/hosts"
      dest: "/etc/hosts2"
  ```

- host directory `"host_dir"`

  Copies a directory from the host to the given location in the artifact.

  ```yaml
  injections:
    - type: "host_dir"
      src: "/etc"
      dest: "/etc2"
  ```
