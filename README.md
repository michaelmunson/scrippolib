# scrippolib

The package manager for [scrippo](../scriptopotamus) packages and scripts. It replaces the
`scrippo -i` / `-ls` / `-rm` commands.

```bash
scrippolib init                         # create .scrippo in this directory
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

```bash
make build      # cargo build
make test
make install    # /usr/local/bin/scrippolib
```

`scrippolib` depends on the `scriptopotamus` crate (the scrippo compiler) by path, so
`../scriptopotamus` must be checked out next to this repository.
