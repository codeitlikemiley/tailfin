# Homebrew formula. Tap this repo:
#   brew tap codeitlikemiley/tailfin https://github.com/codeitlikemiley/tailfin
#   brew install tailfin
#
# sha256 values are filled when a v* tag is cut. Until then, install.sh is the
# supported path — never cargo install --git.
class Tailfin < Formula
  desc "Flight recorder for AI agents"
  homepage "https://github.com/codeitlikemiley/tailfin"
  version "0.1.0"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/codeitlikemiley/tailfin/releases/download/v#{version}/tailfin-aarch64-apple-darwin.tar.gz"
      sha256 "d8380b43bc4423758f54f1b57e73a2a0a609d4880642da21a173d5d44dc167b6"
    else
      url "https://github.com/codeitlikemiley/tailfin/releases/download/v#{version}/tailfin-x86_64-apple-darwin.tar.gz"
      sha256 "4a77e150af0797e913775f1cdf355447c1fa6fc86a0931f1f5b7566a03ef197e"
    end
  end

  on_linux do
    url "https://github.com/codeitlikemiley/tailfin/releases/download/v#{version}/tailfin-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "c24accc4f1a25968adb7534ecf4b09359d7f0997e480b0128034006e7cecc92e"
  end

  def install
    bin.install "tailfin"
  end

  test do
    system "#{bin}/tailfin", "report", "--help"
  end
end
