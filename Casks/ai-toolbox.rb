cask "ai-toolbox" do
  version "1.0.7"

  on_arm do
    sha256 "e4008da9d61c2254a21d0e622362f93a512005f81b338c90f4c288ae0e716032"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.0.7_aarch64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  on_intel do
    sha256 "2006122544f440e9c4764ce6f01bad6f1a7ae4ee9dbd29114672338d388da783"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.0.7_x64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
