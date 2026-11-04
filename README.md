# scrippolib

The package manager for [scrippo](../scriptopotamus) packages and scripts. It replaces the
`scrippo -i` / `-ls` / `-rm` commands, and it now owns `scrippo -n` as `scrippolib new`.

```bash
scrippolib init                         # create .scrippo in this directory
scrippolib new package|script <name> [-g]  # create a starter file (with @info), open the configured editor
scrippolib install <name> [-g]          # registry name, Git URL/repository, or directory
scrippolib update [<name>] [-g]         # one install, or everything in the scope
scrippolib uninstall <name> [-g]
scrippolib list [-g]
scrippolib info <name> [-g]             # installed metadata, else the registry's entry
scrippolib configure [registry=<url>]   # print / set settings
scrippolib --help
scrippolib --version
```

`-g` / `--global` targets `~/.scrippo` (or `SCRIPPO_HOME`); without it, packages go to the
nearest initialized project's `.scrippo/lib` (run `scrippolib init` first) and scripts to the
current directory.

## `<name>`

* A bare name (`shell`) is installed from the configured registry.
* Anything else is a source: a Git URL, a Git checkout, or a plain directory. Use `./name` for
  a directory whose name is also a valid registry name.
* `install --tag <version>` pins a stable Git tag (`1.2.3` or `v1.2.3`).

Sources behave as `scrippo -i` did: every `.scrippo` file whose `@info` declares `--package` or
`--script` is installed, packages are compiled in a sandbox to `<name>.bash`, scripts keep their
source tree and are linked as `<name>.scrippo`, `@hook pre-install` / `post-install` run, and a
failed install or update leaves the previous state intact.

## Registry

The default registry is <https://github.com/michaelmunson/scrippolib-registry>; change it with
`scrippolib configure registry=<url>` (a Git URL or a directory). It is a repository of
`.scrippo` files with `@info` (the default one uses `packages/<name>/<name>.scrippo`). Registries
are not tagged: an entry's version is its `@info --version` on the default branch, and `update`
installs it when that version is higher than the installed one.

Settings are stored in `$SCRIPPO_HOME/scrippolib.config` (override the path with
`SCRIPPOLIB_CONFIG`).

## Build

`scrippolib` is a scrippo script, like `scrippo` itself: [cli/scrippolib.scrippo](cli/scrippolib.scrippo)
declares the commands, options and usage text, and `make build` compiles it to
`target/debug/scrippolib`, a bash script that validates its arguments and then execs
`scrippolib-core`, the Rust binary doing the work (found next to it; override with
`SCRIPPOLIB_CORE`). Both must sit side by side, as `make install` arranges.

```bash
make build      # cargo build + compile the wrapper (needs ../scriptopotamus built: make build there)
make test
make install    # /usr/local/bin/scrippolib and scrippolib-core
```

`scrippolib-core` depends on the `scriptopotamus` crate (the scrippo compiler) by path, so
`../scriptopotamus` must be checked out next to this repository.
