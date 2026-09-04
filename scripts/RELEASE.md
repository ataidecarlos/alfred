# Release Process

## Creating a Release

Alfred uses date-based versioning: `YYYY.MM.DD`

### Steps

1. **Ensure all changes are committed:**
   ```bash
   git status
   git add -A
   git commit -m "Prepare release YYYY.MM.DD"
   ```

2. **Create and push a tag:**
   ```bash
   git tag -a vYYYY.MM.DD -m "Release YYYY.MM.DD"
   git push origin vYYYY.MM.DD
   ```

3. **GitHub Actions will automatically:**
   - Build binaries for all platforms
   - Create release archives
   - Generate checksums
   - Create a GitHub Release with release notes

4. **Verify the release:**
   - Check [GitHub Releases](https://github.com/ataidecarlos/alfred/releases)
   - Verify all artifacts are uploaded
   - Test installation on one platform

## Manual Build (for testing)

### Build for current platform

```bash
cargo build --release
```

### Build release packages

```bash
# Build for all platforms (requires cross-compilation tools)
./scripts/build-release.sh

# Package into release archives
./scripts/package-release.sh
```

## Release Checklist

- [ ] All changes committed to `main`
- [ ] Version number follows `YYYY.MM.DD` format
- [ ] Tag created with `v` prefix (e.g., `v2026.09.04`)
- [ ] GitHub Actions build succeeds
- [ ] All platform artifacts uploaded
- [ ] Checksums generated and verified
- [ ] Release notes updated (auto-generated from commits)
- [ ] Installation tested on at least one platform

## Artifacts

Each release produces:

| Platform | Archive | Checksum |
|----------|---------|----------|
| Linux x64 | `alfred-YYYY.MM.DD-linux-x64.tar.gz` | `alfred-YYYY.MM.DD-linux-x64.tar.gz.sha256` |
| Linux ARM64 | `alfred-YYYY.MM.DD-linux-arm64.tar.gz` | `alfred-YYYY.MM.DD-linux-arm64.tar.gz.sha256` |
| macOS x64 | `alfred-YYYY.MM.DD-macos-x64.tar.gz` | `alfred-YYYY.MM.DD-macos-x64.tar.gz.sha256` |
| macOS ARM64 | `alfred-YYYY.MM.DD-macos-arm64.tar.gz` | `alfred-YYYY.MM.DD-macos-arm64.tar.gz.sha256` |
| Windows x64 | `alfred-YYYY.MM.DD-windows-x64.zip` | `alfred-YYYY.MM.DD-windows-x64.zip.sha256` |

Each archive contains:
- `alfred` (or `alfred.exe`)
- `config.toml.example`
- `prompts/system.md.example`
- `prompts/user.md.example`
- `README.md`

## Hotfix Releases

For urgent fixes:

1. Create a branch from the release tag:
   ```bash
   git checkout -b hotfix/YYYY.MM.DD vYYYY.MM.DD
   ```

2. Apply the fix and commit

3. Create a new patch version:
   ```bash
   git tag -a vYYYY.MM.DD.1 -m "Hotfix: description"
   git push origin vYYYY.MM.DD.1
   ```

## Release Notes

Release notes are auto-generated from commit messages. Use conventional commits for better notes:

- `feat: add new feature` - New Feature
- `fix: fix a bug` - Bug Fix
- `docs: update documentation` - Documentation
- `perf: improve performance` - Performance
- `refactor: refactor code` - Refactoring

## Troubleshooting

### Build fails on GitHub Actions

1. Check the workflow logs
2. Verify all dependencies are in `Cargo.lock`
3. Check for platform-specific code issues

### Artifacts not uploaded

1. Verify GitHub token has write permissions
2. Check artifact names match the workflow
3. Ensure release isn't in draft mode

### Checksum mismatch

1. Rebuild the release
2. Verify no file corruption during upload
3. Check SHA256 calculation is correct
