cask "ai-toolbox" do
  version "1.2.4"

  on_arm do
    sha256 "ec491b50aeac2b6a4b2470aeb70087f3ff7f2304784e6a06277fc8fbf43f0d29"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.2.4_aarch64.dmg"
  end

  on_intel do
    sha256 "3e4f4608ed0391f901513d81db9f696263d531afa24da10c3062572c225ce4a0"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.2.4_x64.dmg"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
