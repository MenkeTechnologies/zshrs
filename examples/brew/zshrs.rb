#
# zshrs.rb — Homebrew formula example showing the `service` stanza
# that lets `brew services start zshrs` autostart zshrs-daemon at
# login. This is a skeleton — once the upstream tap exists, ship a
# real formula in homebrew-tap and drop this file.
#
# Usage once published to a tap:
#
#     brew tap MenkeTechnologies/tap
#     brew install zshrs
#     brew services start zshrs        # daemon starts now + on every login
#     brew services list               # status
#     brew services stop zshrs
#     brew services restart zshrs
#
# brew services materializes this stanza into ~/Library/LaunchAgents/
# (macOS) or ~/.config/systemd/user/ (Linux) automatically — the same
# launchd plist / systemd unit shipped under examples/launchd/ +
# examples/systemd/, so you don't have to install those by hand.
#
class Zshrs < Formula
  desc "Compiled zsh-superset shell + cache/recorder daemon"
  homepage "https://github.com/MenkeTechnologies/zshrs"
  url "https://github.com/MenkeTechnologies/zshrs/archive/refs/tags/v0.10.7.tar.gz"
  sha256 "REPLACE_WITH_TARBALL_SHA256"
  license "MIT"
  head "https://github.com/MenkeTechnologies/zshrs.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: ".")
    system "cargo", "install", *std_cargo_args(path: "daemon")
  end

  service do
    run [opt_bin/"zshrs-daemon"]
    keep_alive crashed: true, successful_exit: false
    log_path   var/"log/zshrs/daemon.log"
    error_log_path var/"log/zshrs/daemon.err.log"
    working_dir Dir.home
    environment_variables ZSHRS_QUIET_FIRST_RUN: "1"
    process_type :interactive
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/zshrs --version")
    assert_match version.to_s, shell_output("#{bin}/zshrs-daemon --version")
  end
end
