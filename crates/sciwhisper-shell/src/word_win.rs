//! Native Word insertion on Windows: OMML InsertXML, then UnicodeMath BuildUp.
//! Compiled only for Windows; other platforms never call this module.

use std::fs;
use std::process::Command;

use sciwhisper_core::Node;

use crate::error::{Error, Result};

pub fn insert_omml(omml: &str, ast: &Node) -> Result<()> {
    let xml = sciwhisper_core::word_insert_xml(ast);
    let tmp = std::env::temp_dir().join("sciwhisper-omml.xml");
    fs::write(&tmp, xml.as_bytes())?;
    let linear = sciwhisper_core::render(ast, sciwhisper_core::Renderer::Unicode);
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$xmlPath = '{xml_path}'
$linear = @'
{linear}
'@
$word = [Runtime.InteropServices.Marshal]::GetActiveObject('Word.Application')
try {{
  $xml = Get-Content -Raw -Encoding UTF8 $xmlPath
  $word.Selection.InsertXML($xml) | Out-Null
  'omml'
}} catch {{
  $rng = $word.Selection.Range
  $rng.Text = $linear
  $null = $rng.OMaths.Add($rng)
  $rng.OMaths.Item(1).BuildUp()
  'buildup'
}}
"#,
        xml_path = tmp.display().to_string().replace('\'', "''"),
        linear = linear.replace("'", "''"),
    );
    let _ = omml;
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .output()
        .map_err(|e| Error::Message(e.to_string()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(Error::Message(format!("Word COM failed: {err}")));
    }
    Ok(())
}
