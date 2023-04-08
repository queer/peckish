# peckish

peckish (case-sensitive) is a tool for repackaging Linux software artifacts.

For example, suppose you're an application developer. You just made something
cool and want to distribute it. However, packaging is *hard*. Different package
formats do things differently -- ex. Arch has `x86_64` and `any` as architectures,
but Debian has over a dozen and calls x86_64 `amd64` -- and it's hard to
remember all the specifics. This is compounded by having to figure out the
appropriate CLI flags for each package format. How many people can write a
[valid `tar` command](https://xkcd.com/1168/) on the first try? :P

peckish solves this. Instead of mangling your files with various arcane tools,
spinning up Docker containers or VMs to try to build packages and make sure
they install or are even valid, and all the other pains, you can just write a
basic YAML file, and you're done! peckish does the rest, *without* shelling out
to distro-specific tools!

For example, peckish lets you take a program you wrote and turn it into:

- a tarball
- an installable Debian package
- an installable Arch Linux package
- a runnable Docker image
- an installable RPM package

This problem is fairly common:

> It seems more and more common, at least in the self-hosted community, to
> provide two methods of installations: Docker or a shell script installer. It
> seems that many new projects don’t even bother with a native package manager,
> such as dpkg or rpm, which makes me reluctant to use them.
>
> https://yotam.net/posts/the-audacity-of-piping-curl-to-bash/

> It's easier to compile the linux kernel from scratch than it is to build a
> debian package from scratch.
>
> It's mind-blowing how complicated it is; at the end of the day it's just an
> archive. I hate having to do it.
>
> https://news.ycombinator.com/item?id=34787228

> The impact of having frictionless package building cannot be understated. I'm
> publishing Arch Linux packages for all my applications because it takes just
> a few minutes to write up a PKGBUILD. Then one time, I tried providing a
> Debian package as well, but I gave up after several hours of trying to get
> through all the bureaucracy of the tooling.
>
> https://news.ycombinator.com/item?id=34923960

> [...] I've created my own RPM and DEB packages in the past as well; but, at
> least when I did it years ago, it wasn't as effective as a PKGBUILD on arch.
>
> https://news.ycombinator.com/item?id=35014171

> Official documentation is a problem. It is aimed at Debian maintainers
> building a fully policy compliant package that could be accepted to the
> official repository. You don't need any of that crap for making a .deb
> package. It puts off people from creating Debian packages for their own
> programs or private use.
>
> https://news.ycombinator.com/item?id=34788350

Some currently-sparse docs about the different artifacts and producers can be
found here:

https://github.com/queer/peckish/tree/mistress/docs

### caveats

- peckish is `0.x.y`! Treat it accordingly. Maybe don't run it in prod.
- Documentation is hard, and I am not the best at it. Some questions may only
  be able to be answered by reading the source or opening an issue. Reporting
  missing and/or broken docs helps everyone!
- peckish is not a build system or a package manager. It doesn't care about how
  your code is built or installed, just about getting it from one package
  format to another.
- peckish **cannot** guarantee all dependencies are placed into the package
  correctly. It's up to you to make sure your package is statically linked, or
  that all dependencies are included in the package.
- peckish is primarily tested for my use-cases. Please please please please
  report bugs!


## usage

Create a `peckish.yaml` file in the root of your project.

```yaml
# whether to run as a pipeline, ie each artifact output is the input to the
# next producer. defaults to `false` if not specified.
pipeline: false

# metadata about the package. required, even if you're only producing a file or
# a tarball. this is because it's just easier than trying to play the "is there
# enough metadata to build the package" game.
metadata:
  name: "my-cool-pkg"
  # many distros want versions that end in -#, which is a revision number for
  # the package. this is required if you are producing an Arch/Deb/RPM/similar
  # package.
  version: "0.1.0-1"
  description: "a package"
  # suggested format: "me <me@example.com>"
  author: "me"
  # the architecture of the system the package is built for. this is usually
  # the same as the architecture of the system you're building on. will be
  # automatically set to the correct value for the target package format, ex.
  # x86_64 -> amd64 for debian.
  arch: "amd64"
  license: "Apache-2.0"

# the artifact being used as input to the pipeline.
input:
  name: "some file"
  type: "file"
  paths:
  - "./path/to/file"

# the producers being used as pipeline outputs. see `docs/` for more info about
# each producer.
output:
  - name: "tarball"
    type: "tarball"
    path: "./whatever.tar"

  - name: "debian package"
    type: "deb"
    path: "./whatever.deb"
```

### supported package formats

The goal is to be able to support all of these! Formats not listed may end up
on the list :D

- [x] file `"file"`
- [x] arch package `"arch"`
- [x] tarball `"tarball"`
- [x] debian package `"deb"`
- [x] docker image `"docker"`
- [x] rpm package `"rpm"`
- [ ] appimage `"appimage"`
  - https://crates.io/crates/appimage
    - Missing read support...
- [ ] Filesystem and VM images (tbd)
  - Working on `libguestfs` bindings: https://github.com/queer/libguestfs-rs
- [ ] Flatpak?
  - More research needed

### library

```rust
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

// pipelines
use peckish::prelude::pipeline::*;
use peckish::prelude::*;

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
```

## concepts

peckish is built around the concepts of *artifacts* and *producers*.

Artifacts are some sort of data that exists on your system that can be
packaged; artifacts themselves do not contain any of that data, just metadata.
For example, a `FileArtifact` is a list of paths to files on your system. A
`TarballArtifact` is a path to a tarball. A `DebArtifact` is a path to a
`.deb` file. So on and so forth.

Producers are a bit more interesting. Producers are the things that actually
do the packaging. They take an artifact as input and produce a new artifact
as output. For example, a `TarballProducer` takes a `FileArtifact` as input
and produces a `TarballArtifact` as output. A `DebProducer` takes a
`TarballArtifact` as input and produces a `DebArtifact` as output.

peckish artifacts and producers are centred around the idea of an in-memory
filesystem. Rather than having to mangle things on the disk, peckish moves
everything into memory, manipulates it, then flushes it back to disk. This
allows for trivial manipulation of software artifacts, as changing them is
simply injecting some changes into the in-memory filesystem and repackaging
with the metadata in the producer. No knowledge of the previous artifact is
needed beyond its in-memory filesystem representation.

## license

Copyright 2023-present amy null

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
