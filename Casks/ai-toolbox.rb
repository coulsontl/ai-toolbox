cask "ai-toolbox" do
  version "0.9.8"

  on_arm do
    sha256 "536f4adf55d8c81afc77964deb79925ba99a8df6c53d31bf89ed31d5085d171b"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.9.8_aarch64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  on_intel do
    sha256 "3b7c2cd6fe87e4b510f1ac42df59aa73352299fa834fe8968e81355cd965f3ae"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.9.8_x64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
