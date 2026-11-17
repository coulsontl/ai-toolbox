cask "ai-toolbox" do
  version "1.2.3"

  on_arm do
    sha256 "b33b58ab4c982b3adc34b4af208448c1cc10b6de735ef47fa9b61aa468a771a0"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.2.3_aarch64.dmg"
  end

  on_intel do
    sha256 "c62484b4cf2a4021dc6879d45ccf2140f276f04a0be15e7911766170e7de94e0"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.2.3_x64.dmg"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
