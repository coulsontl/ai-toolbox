cask "ai-toolbox" do
  version "0.9.0"

  on_arm do
    sha256 "a6a909cb14bbaa2c1aa2dd98b438950f47100e9d913c6519c0de70e8073a512a"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.9.0_aarch64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  on_intel do
    sha256 "f41e2205df949b07b0d9e7e93927aab0a0ee6837f1d8e533a188e6605d636f0a"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.9.0_x64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
