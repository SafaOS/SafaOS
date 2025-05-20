# Contributing
## Configuring Rust Analyzer
Unfortunately `rust-analyzer` does some weird shenaganis to discover projects and the build system being at the root of the repo breaks it so you have to configure it to pick up the packages you want to edit,

Sadly `rust-analyzer.toml` is not stable yet so you have to configure it for your editor yourself you can use: [rust-analyzer.linkedProjects](https://rust-analyzer.github.io/book/configuration.html#linkedProjects), add the paths to the Cargo.toml(s) you want to edit for example i edit everything and i am using [Zed](https://zed.dev/) so my `.zed/settings.json` looks like this:
```
{
  "lsp": {
    "rust-analyzer": {
      "initialization_options": {
        "linkedProjects": [
          "crates/kernel/Cargo.toml",
          "Cargo.toml",
          "crates-user/safa-binutils/Cargo.toml",
          "crates-user/Shell/Cargo.toml",
          "crates-user/safa-tests/Cargo.toml",
        ]
      }
    }
  }
}
```
for more information see rust analyzer's [configuration docs](https://rust-analyzer.github.io/book/configuration.html)

## Commits
Commit messages is preferred to be formatted like this:
```
HEADER: BODY
```
There is no rules regarding the `HEADER` or the `BODY`'s format, for now this is not enforced but it is preferred to do so.

## Contributing To The Userspace
To contribute to the userspace you can either create a new rust crate (project/package) in `crates-user`, then it will automatically picked by the buildsystem (`safa-helper`).

or you can contribute to an existing crate such as the [safa-binutils](/crates-user/safa-binutils), or the [Shell](https://github.com/SafaOS/Shell).

## Contributing To The Kernel
You can modify the kernel code in `crates/kernel`.
