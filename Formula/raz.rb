# Homebrew formula. Tap this repo:
#   brew tap goldcoders/raz https://github.com/goldcoders/raz
#   brew install raz
#
# sha256 values are filled when a v* tag is cut. Until then, install.sh is the
# supported path — never cargo install --git.
class Raz < Formula
  desc "Flight recorder for AI agents"
  homepage "https://github.com/goldcoders/raz"
  version "0.1.0"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/goldcoders/raz/releases/download/v#{version}/raz-aarch64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    else
      url "https://github.com/goldcoders/raz/releases/download/v#{version}/raz-x86_64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  on_linux do
    url "https://github.com/goldcoders/raz/releases/download/v#{version}/raz-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  end

  def install
    bin.install "raz"
  end

  test do
    system "#{bin}/raz", "report", "--help"
  end
end
