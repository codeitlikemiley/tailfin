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
      sha256 "61035dd20f724e24d6b8377a82e1ce4447b0fe96e93f4c9242ef028175dd6825"
    else
      url "https://github.com/codeitlikemiley/tailfin/releases/download/v#{version}/tailfin-x86_64-apple-darwin.tar.gz"
      sha256 "7c17ac5b03c5c212053bbfcad3e8c86d72bad7959be9f84afaecb1c08360b41b"
    end
  end

  on_linux do
    url "https://github.com/codeitlikemiley/tailfin/releases/download/v#{version}/tailfin-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "53c331ec97fba9eb11d97b6b0a7c61dfc63fd062c4643e89259f019afac88745"
  end

  def install
    bin.install "tailfin"
  end

  test do
    system "#{bin}/tailfin", "report", "--help"
  end
end
