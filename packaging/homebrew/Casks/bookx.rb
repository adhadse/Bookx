# Homebrew cask template for the Bookx disk image that .github/workflows/package-native.yml builds.
# Goes into adhadse/homebrew-bookx as Casks/bookx.rb. NOT TESTED (no macOS here), and a cask needs a *signed and
# notarized* .dmg: Homebrew is removing casks that Gatekeeper rejects. Needs an Apple Developer ID
# (docs/macos-port.md, "Owner").
#
# After a release fill in `version` and both checksums:
#   shasum -a 256 Bookx-<version>-arm64.dmg Bookx-<version>-x86_64.dmg
cask "bookx" do
  arch arm: "arm64", intel: "x86_64"

  version "0.3.0"
  sha256 arm:   "0000000000000000000000000000000000000000000000000000000000000000",
         intel: "0000000000000000000000000000000000000000000000000000000000000000"

  url "https://github.com/adhadse/Bookx/releases/download/v#{version}/Bookx-#{version}-#{arch}.dmg"
  name "Bookx"
  desc "E-book library, reader and editor"
  homepage "https://bookx.adhadse.com"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: ">= :sonoma"

  app "Bookx.app"
  binary "#{appdir}/Bookx.app/Contents/MacOS/bookx-cli", target: "bookx-cli"

  zap trash: [
    "~/Library/Application Support/com.adhadse.Bookx",
    "~/Library/Caches/com.adhadse.Bookx",
    "~/Library/Preferences/com.adhadse.Bookx.plist",
    "~/Library/Saved Application State/com.adhadse.Bookx.savedState",
  ]
end
