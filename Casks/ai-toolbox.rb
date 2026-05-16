cask "ai-toolbox" do
  version "0.9.1"

  on_arm do
    sha256 "69672931e564d9fcd71b5bcb8f29c141e2ebd39c2b61d835244e7a68e7aa679b"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.9.1_aarch64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  on_intel do
    sha256 "1eac8752af767ed82d6f140811e41cd6bdc15ae9b0c7ed6c5ecac8523a063517"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.9.1_x64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
