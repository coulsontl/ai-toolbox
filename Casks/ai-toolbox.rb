cask "ai-toolbox" do
  version "1.1.7"

  on_arm do
    sha256 "1c38e47403778d0d3c8e8cd54256005ab739ad8fdfa189f0a2f3926270dbb684"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.1.7_aarch64.dmg"
  end

  on_intel do
    sha256 "c716820a1eb474f4104b4f590afc19098770ae5fa55754ff338e470320380998"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.1.7_x64.dmg"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
