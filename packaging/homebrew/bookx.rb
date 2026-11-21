# Homebrew formula for Bookx: builds from source. Goes into the tap repository adhadse/homebrew-bookx
# as Formula/bookx.rb (see README.md here). NOT TESTED: no macOS was available when this was written.
#
# Before the first release: set `url` to the tag tarball and `sha256` to
#   curl -L <url> | shasum -a 256
class Bookx < Formula
  desc "E-book library, reader and editor (GTK 4 and libadwaita), written in Rust"
  homepage "https://bookx.adhadse.com"
  url "https://github.com/adhadse/Bookx/archive/refs/tags/v0.3.0.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "GPL-3.0-or-later"
  head "https://github.com/adhadse/Bookx.git", branch: "main"

  depends_on "pkgconf" => :build
  depends_on "rust" => :build
  depends_on "adwaita-icon-theme"
  depends_on "espeak-ng"
  depends_on "glib"
  depends_on "gtk4"
  depends_on "gtksourceview5"
  depends_on "libadwaita"

  def install
    # the library is `bookx`, the command line tool is `bookx-cli`
    system "cargo", "install", *std_cargo_args(path: ".")
    system "cargo", "install", *std_cargo_args(path: "crates/bookx-cli")

    # GTK finds schemas, icons and espeak-ng data through these variables
    libexec.install bin/"bookx"
    (bin/"bookx").write_env_script libexec/"bookx",
      GSETTINGS_SCHEMA_DIR: "#{HOMEBREW_PREFIX}/share/glib-2.0/schemas",
      XDG_DATA_DIRS:        "#{share}:#{HOMEBREW_PREFIX}/share:/usr/local/share:/usr/share",
      ESPEAK_DATA_PATH:     "#{Formula["espeak-ng"].opt_share}/espeak-ng-data"

    icons = "data/icons/hicolor"
    (share/"icons/hicolor/scalable/apps").install "#{icons}/scalable/apps/com.adhadse.Bookx.svg"
    (share/"icons/hicolor/symbolic/apps").install "#{icons}/scalable/apps/com.adhadse.Bookx-symbolic.svg"
    (share/"applications").install "data/com.adhadse.Bookx.desktop"
    (share/"metainfo").install "data/com.adhadse.Bookx.metainfo.xml"
    doc.install "README.md", "CHANGELOG.md", "NOTICE.md"
  end

  def caveats
    <<~EOS
      Bookx is not signed or notarized by this formula; it runs the binary Homebrew built on your Mac.
      Read aloud with the natural voices and audiobooks need an audio sink that is not built for macOS yet
      (docs/macos-port.md in the source tree); everything else is expected to work.
    EOS
  end

  test do
    assert_match "Usage", shell_output("#{bin}/bookx-cli --help")
  end
end
