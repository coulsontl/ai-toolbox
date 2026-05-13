cask "ai-toolbox" do
  version "0.9.0"

  on_arm do
    sha256 "dabe49a0a77bdfc00134e99edaf1fda5e076d44f76ceb45a9280cf41e07b40bf"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.9.0_aarch64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  on_intel do
    sha256 "4cc73ef53b28fa1108cadda7f857209b1f208d79aef51558f5a124cfe7a200fe"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.9.0_x64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
