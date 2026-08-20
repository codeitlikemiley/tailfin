# Homebrew formula. Tap this repo:
#   brew tap codeitlikemiley/tailfin https://github.com/codeitlikemiley/tailfin
#   brew install tailfin
#
# sha256 values are filled when a v* tag is cut. Until then, install.sh is the
# supported path — never cargo install --git.
class Tailfin < Formula
  desc "Flight recorder for AI agents"
  homepage "https://github.com/codeitlikemiley/tailfin"
  version "0.1.2"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/codeitlikemiley/tailfin/releases/download/v#{version}/tailfin-aarch64-apple-darwin.tar.gz"
      sha256 "c794fb8fa698d9fc9a258a667fc4eb31f838ee1c9f8eb83966378169508293f1"
    else
      url "https://github.com/codeitlikemiley/tailfin/releases/download/v#{version}/tailfin-x86_64-apple-darwin.tar.gz"
      sha256 "df369942c2f06f6d6727f73d7770559c776732177d6e289d67e48b807c5f9847"
    end
  end

  on_linux do
    url "https://github.com/codeitlikemiley/tailfin/releases/download/v#{version}/tailfin-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "e52edf4009c36f3b85a5598790d99b26a249a231f6c9e81366a26ff8f8a57638"
  end

  def install
    bin.install "tailfin"
  end

  test do
    system "#{bin}/tailfin", "report", "--help"
  end
end
