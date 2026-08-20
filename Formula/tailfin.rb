# Homebrew formula. Tap this repo:
#   brew tap codeitlikemiley/tailfin https://github.com/codeitlikemiley/tailfin
#   brew install tailfin
#
# sha256 values are filled when a v* tag is cut. Until then, install.sh is the
# supported path — never cargo install --git.
class Tailfin < Formula
  desc "Flight recorder for AI agents"
  homepage "https://github.com/codeitlikemiley/tailfin"
  version "0.1.1"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/codeitlikemiley/tailfin/releases/download/v#{version}/tailfin-aarch64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    else
      url "https://github.com/codeitlikemiley/tailfin/releases/download/v#{version}/tailfin-x86_64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  on_linux do
    url "https://github.com/codeitlikemiley/tailfin/releases/download/v#{version}/tailfin-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  end

  def install
    bin.install "tailfin"
  end

  test do
    system "#{bin}/tailfin", "report", "--help"
  end
end
