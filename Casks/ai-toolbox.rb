cask "ai-toolbox" do
  version "1.0.5"

  on_arm do
    sha256 "96bf84dcdbdbc93d382f8573c388e19e8756ad21b75be4132538da9b37973ce9"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.0.5_aarch64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  on_intel do
    sha256 "a6de469df43c6f14038a5af9ecf0d6c80cec7bfab12056ce81cf38a61dbf6183"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.0.5_x64.dmg",
        verified: "github.com/coulsontl/ai-toolbox/"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
