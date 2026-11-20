# Contributing
## AI Policy
Any PR made must discuss where is the usage of AI, in case no AI generated code was used then this is not necessary, a comment before every AI generated is preferred, non-AI generated PRs are preferred, the PR may be rejeceted for any usage of AI if deemed unnecessary.

## Commits
Commit messages is preferred to be formatted like this:
```
HEADER: BODY
```
There is no rules regarding the `HEADER` or the `BODY`'s format, for now this is not enforced but it is preferred to do so.

## Including Files in the ramdisk
Just copy the files to the [`ramdisk-include`](/ramdisk-include) directory, and then build.

The ISO then should contain the file to the same exact path thats relative to the `ramdisk-include` directory to a path relative to the `sys:/` directory (sys:/ is where the ramdisk is mounted).

## Contributing To The Userspace
To contribute to the userspace you can either create a new rust crate (project/package) in `safa-userspace`, then it will automatically picked by the buildsystem (`safa-helper`).

or you can contribute to an existing crate such as the [safa-binutils](/safa-userspace/safa-binutils), or the [Shell](https://github.com/SafaOS/Shell).

## Contributing To The Kernel
You can modify the kernel code in `safa-*core` and `safa-mods`, you may find the old kernel in `safa-core`.
**TODO** more detailed documentation of the SafaOS module system and the new WIP `safa-acore` kernel.
