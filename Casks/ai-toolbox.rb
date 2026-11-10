cask "ai-toolbox" do
  version "1.2.2"

  on_arm do
    sha256 "285359d61eaf905c4b75ffff8ca9544b9fe35358b4a3af90a0de06da1993cfe8"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.2.2_aarch64.dmg"
  end

  on_intel do
    sha256 "79edbc0d50a8552d10b98912814f0af6bbd436086cab2a49fb54fd2613a67860"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.2.2_x64.dmg"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
