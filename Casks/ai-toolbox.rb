cask "ai-toolbox" do
  version "0.7.8"

  on_arm do
    sha256 "01cf67d2541d3d600590d2d0a27f453c44b36054c937b899b2dbaed9d1c77dca"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.7.8_aarch64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  on_intel do
    sha256 "a0c2cd312bcd11c5d00f075fa35dcbc5d95e9dfb61cf86560991dcf1b53ae640"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_0.7.8_x64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
