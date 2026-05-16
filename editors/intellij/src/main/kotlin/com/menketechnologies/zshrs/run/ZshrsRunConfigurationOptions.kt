package com.menketechnologies.zshrs.run

import com.intellij.execution.configurations.LocatableRunConfigurationOptions

class ZshrsRunConfigurationOptions : LocatableRunConfigurationOptions() {
    var scriptPath: String? by string()
    var scriptArgs: String? by string()
    var interpreterArgs: String? by string()
    var workingDirectory: String? by string()
    var noRcs: Boolean by property(false)          // -f / --no-rcs
    var xtrace: Boolean by property(false)         // -x
    var verbose: Boolean by property(false)        // -v
    var disasm: Boolean by property(false)         // --disasm
    var dumpAst: Boolean by property(false)        // --dump-ast (-)
    var compatMode: String? by string()            // --zsh / --bash / --ksh / --posix
}
