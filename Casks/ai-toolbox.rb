cask "ai-toolbox" do
  version "1.0.3"

  on_arm do
    sha256 "cd20dcd9cfde9f00a03bc072a9334d989b0a931f225bdccb818ad02d7e5ea092"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.0.3_aarch64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  on_intel do
    sha256 "2feb796145ce5adcaefb3bbd5f5330c7afe6460e5907abe79bb2c62f605830d0"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.0.3_x64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
