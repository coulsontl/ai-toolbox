cask "ai-toolbox" do
  version "1.0.0"

  on_arm do
    sha256 "6bf60492f31df6081fe19957392d7129c40d6e9f97d474a11b93f8da43ba6fdf"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.0.0_aarch64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  on_intel do
    sha256 "b6d3263138cbeb51adaed2bea3d26cbee1571de2122bc899a342fd62081680c0"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.0.0_x64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
