# Homebrew Cask for Zalo-next (for a personal tap, not homebrew/cask).
#
# Publish: create the GitHub repo `minhkhang1008/homebrew-tap`, copy this file to
# `Casks/zalo-next.rb` there, then users run:
#
#   brew install --cask minhkhang1008/tap/zalo-next
#
# Renamed from the `ctdozlite-next` cask: keep a stub `Casks/ctdozlite-next.rb` in the tap that
# calls `deprecate! date: "2026-11-17", because: :renamed` (or add `tap_migrations.json`
# {"ctdozlite-next": "zalo-next"}) so `brew upgrade` still finds installs of the old name.
#
# On each release update `version` and both `sha256` values from the release's SHA256SUMS.txt.
# The app is ad-hoc signed (not notarized), so postflight clears the quarantine flag that
# Homebrew adds; otherwise Gatekeeper would block the first launch.
cask "zalo-next" do
  arch arm: "arm64", intel: "x86_64"

  version "0.2.1"
  sha256 arm:   "99874ee5735a1398afa183f524e4ebf0d57bf9379c6851c6e507ffb3c9348d11",
         intel: "23031888a21e2aa69fedcfe9ee6b4090769537802a5ba1a9e26dc7c6a6b9cf2c"

  url "https://github.com/minhkhang1008/zalo-next/releases/download/v#{version}/zalo-next-#{version}-macos-#{arch}.dmg"
  name "Zalo-next"
  desc "Lightweight native Zalo desktop client written in Rust"
  homepage "https://github.com/minhkhang1008/zalo-next"

  depends_on macos: :big_sur

  # The app updates itself from GitHub Releases.
  auto_updates true

  app "Zalo-next.app"

  postflight_steps do
    run "/usr/bin/xattr", args: ["-cr", "{{appdir}}/Zalo-next.app"]
  end

  zap trash: "~/Library/Application Support/ctdozlite"
end
