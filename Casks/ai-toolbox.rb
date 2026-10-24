cask "ai-toolbox" do
  version "1.2.1"

  on_arm do
    sha256 "bccead714e2b59be6dd4b135bc5add7549e26b34b0f09bc33ffcb33539929ed4"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.2.1_aarch64.dmg"
  end

  on_intel do
    sha256 "7abe137ca882dce0ec30d08c0520c58a235530d05c9370afc9ec687970948760"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.2.1_x64.dmg"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
