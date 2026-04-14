cask "ai-toolbox" do
  version "0.8.1_beta1"

  on_arm do
    sha256 "f7e810a5633c72161a2443b0fff24eab3c3b4ed17a87a56fc777cbe94909c11b"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.8.1_aarch64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  on_intel do
    sha256 "7adfbe98fe63c2318300732ac8ea343e5f629b0d88da1800402a7e62ca08e06b"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.8.1_x64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
