# Portable local-key backup

Portable backup is a versioned envelope containing Argon2id parameters, random salt, XChaCha20-Poly1305 nonce, ciphertext of the random master key, and authenticated format metadata. It deliberately excludes the Windows DPAPI blob, so it can be moved to another computer.

Restoring requires the original master password. Wrong passwords, altered metadata/ciphertext, and unknown format versions are rejected. A forgotten master password cannot be recovered by JARVIS; keep a secure backup of the password and encrypted envelope.
