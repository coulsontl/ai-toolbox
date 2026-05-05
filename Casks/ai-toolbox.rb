cask "ai-toolbox" do
  version "0.8.7"

  on_arm do
    sha256 "934fbe2d0d65a8705f3d49cdf56d2356c89b4a2e59494e9ca83edb05be21f83e"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.8.7_aarch64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  on_intel do
    sha256 "83f255886c138b60cf04a59fe31b2694299e3011fc380950543d14ec09632a90"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.8.7_x64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
