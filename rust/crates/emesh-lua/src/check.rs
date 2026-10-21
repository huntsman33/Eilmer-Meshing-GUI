//! Compile-check generated scripts with an embedded Lua 5.4 (the version Eilmer uses), without
//! running them: Eilmer's geometry API exists only inside `lmr`, so this catches syntax errors
//! before `lmr` is started.

/// Ok if the script compiles as Lua 5.4.
pub fn compiles(script: &str, name: &str) -> Result<(), String> {
    let lua = mlua::Lua::new();
    lua.load(script).set_name(name).into_function().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn syntax_errors_are_reported() {
        assert!(super::compiles("x = 1", "ok").is_ok());
        assert!(super::compiles("x = = 1", "bad").is_err());
    }
}
