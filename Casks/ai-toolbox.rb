cask "ai-toolbox" do
  version "0.7.7"

  on_arm do
    sha256 "30b0945ab2d924be1855f066735ec40dbe0bbac115bbb94e1dce86e9e9310e19"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_#{version}_aarch64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  on_intel do
    sha256 "7477147ed78bdc117599b3de9e8dfb361bcabba0eae79c440dc1f9ade962708e"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_#{version}_x64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
