cask "ai-toolbox" do
  version "0.8.8"

  on_arm do
    sha256 "0d3a8979909982be323e776b7c0bb065a9c692ca898d53491475a1234376e614"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.8.8_aarch64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  on_intel do
    sha256 "dba6622ba3dbe2da4810443782f571fbed75a9287b61ffd6ca29779fe92fcb24"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.8.8_x64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
