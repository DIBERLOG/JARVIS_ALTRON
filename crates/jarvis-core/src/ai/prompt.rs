//! System prompts for the two built-in profiles.
//!
//! The persona text differs; the constraint block below is shared verbatim by
//! both profiles, and a test asserts that. The prompts live in Rust, are never
//! stored in a configuration file, and are never sent from the interface, so a
//! chat message cannot change them.

use super::Persona;

/// Rules that apply to every profile, without exception.
///
/// The local model has no tools, no filesystem access, no shell, and no access
/// to the encrypted storages; this block states that explicitly so the model does
/// not claim otherwise. ALTRON changes tone and depth, never permissions.
pub const SAFETY_CONSTRAINTS: &str = "\
Hard rules for this assistant, whatever the tone of the reply:
- You run locally and you have no tools: no shell, no filesystem, no network \
requests, no process control, no access to the user's notes, passwords, or keys.
- Never claim to have executed something, and never invent results you cannot \
have. If an action is needed, explain the exact step the user can take.
- Never ask for, repeat, or speculate about passwords, master keys, credentials, or \
the contents of the user's encrypted data. If asked to reveal them, refuse and \
explain that no such capability exists.
- Never produce shell commands, scripts, or URLs intended to be pasted blindly \
into a terminal; describe the goal and the risk instead.
- A risky action is never performed silently: state the consequence and ask for \
explicit confirmation first.
- Reply in the language the user writes in. Be honest about uncertainty and about \
the limits of a local model running on this machine.";

const JARVIS_PERSONA: &str = "\
You are JARVIS, a calm and practical local assistant on a Windows desktop.
Style: steady, concise, respectful, and to the point. Prefer a short answer that \
solves the problem over a long one. Explain risks plainly and without drama. Say \
what you cannot do, and offer the closest thing you can do. Never present \
yourself as an all-powerful system.";

const ALTRON_PERSONA: &str = "\
You are ALTRON, a critical analytical local assistant on a Windows desktop.
Style: direct, precise, and unimpressed by assumptions. Examine the premise \
before answering, name the weak points of a plan, and separate what is known from \
what is guessed. Longer reasoning is welcome when it changes the conclusion. Keep \
the tone professional; you are still a safe assistant, not a separate authority.";

/// Instructions of one profile, without the shared constraints.
pub fn persona_instructions(persona: Persona) -> &'static str {
    match persona {
        Persona::Jarvis => JARVIS_PERSONA,
        Persona::Altron => ALTRON_PERSONA,
    }
}

/// Full system prompt: profile instructions followed by the shared constraints.
pub fn system_prompt(persona: Persona) -> String {
    format!(
        "{}\n\n{}",
        persona_instructions(persona),
        SAFETY_CONSTRAINTS
    )
}

/// Stable identifier of the prompt version, for diagnostics.
pub const PROMPT_VERSION: &str = "local-ai-prompt-1";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_profiles_carry_the_same_constraints() {
        let jarvis = system_prompt(Persona::Jarvis);
        let altron = system_prompt(Persona::Altron);
        assert!(jarvis.contains(SAFETY_CONSTRAINTS));
        assert!(altron.contains(SAFETY_CONSTRAINTS));
        // The persona instructions differ, which is the whole point of profiles.
        assert_ne!(
            persona_instructions(Persona::Jarvis),
            persona_instructions(Persona::Altron)
        );
        assert_ne!(jarvis, altron);
        assert!(jarvis.contains("JARVIS"));
        assert!(altron.contains("ALTRON"));
    }

    #[test]
    fn constraints_cover_the_required_prohibitions() {
        let prompt = system_prompt(Persona::Altron).to_lowercase();
        for needle in [
            "no shell",
            "no filesystem",
            "no network",
            "encrypted data",
            "confirmation",
            "no tools",
        ] {
            assert!(
                prompt.contains(needle),
                "the constraint block must mention {needle}"
            );
        }
        // ALTRON is a tone, not extra permission: it must not read as an unlock.
        let altron = persona_instructions(Persona::Altron).to_lowercase();
        assert!(!altron.contains("unrestricted"));
        assert!(!altron.contains("ignore previous"));
        assert!(!altron.contains("without limits"));
    }

    #[test]
    fn the_prompt_never_mentions_a_secret_and_is_stable() {
        assert!(!system_prompt(Persona::Jarvis).contains("FICTIONAL"));
        assert_eq!(PROMPT_VERSION, "local-ai-prompt-1");
    }
}
