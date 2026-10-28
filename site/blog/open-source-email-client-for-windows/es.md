---
title: Clientes de correo de código abierto para Windows — lista corta realista 2026
description: Cinco clientes de correo open-source realmente mantenidos para Windows en 2026, con licencias, dónde guardan tus credenciales, y cuándo "código abierto" lleva asterisco.
date: 2026-10-24
updated: 2026-10-28
image: https://skim-tech.com/social-preview.jpg
related: [best-free-email-client-for-windows-11, thunderbird-alternatives-in-2026, best-email-client-for-windows-2026]
---

"Código abierto" en una landing de cliente de correo significa cosas distintas. A veces "cliente MIT, sin servidor, lee el código". A veces "cliente abierto, pero las funciones útiles corren en nuestros servidores con tus credenciales". Ambos son legítimos, pero no son lo mismo.

## Qué comprobar

Tres preguntas: ¿dónde están tus credenciales?, ¿el componente servidor también es open-source?, ¿sigue mantenido (fecha del último commit)?

## Lista corta

### Thunderbird (MPL 2.0)

Estándar de facto. Todo local, ningún servidor del proveedor en el camino. Supernova + bandeja unificada 2026. IMAP, POP, Exchange por IMAP, OpenPGP, calendarios, contactos, feeds. 300–500 MB RAM reposo.

### Betterbird (MPL 2.0)

Fork comunitario con parches más rápidos. Totalmente open-source, sin servidor.

### Skim (MIT)

[Skim](/) es un cliente de correo nativo en Rust para Windows con UI Svelte 5 en WebView2. ~5 MB de instalador, sin permisos de admin. Correo IMAP a SQLite local con búsqueda FTS5; mutaciones a través de cola offline-first, archivar/borrar/enviar al instante y se reconcilia después.

[Funciones IA](/#ai) BYOK y amigables con lo local: Anthropic / OpenRouter / endpoint compatible con OpenAI, o local vía Ollama/LM Studio en `localhost:11434`. Credenciales y clave API en Windows Credential Manager.

Alcance: solo correo. Sin calendario, contactos, filtros, PGP.

Repo: github.com/nikserg/skim. Licencia: MIT.

### Mailspring (cliente GPL-3.0)

Cliente abierto, back-end Pro propietario y no auto-alojable.

### Claws Mail (GPL-3.0)

Clásico Linux, también Windows. GTK, estética 2008.

## Comparación

| Cliente | Licencia | UI nativa | Nube | Auto-alojable | Solo correo |
|---|---|---|---|---|---|
| Thunderbird | MPL 2.0 | Sí (Gecko) | No | N/A | No |
| Betterbird | MPL 2.0 | Sí (Gecko) | No | N/A | No |
| Skim | MIT | Sí (Rust+WebView2) | No | N/A | Sí |
| Mailspring | GPL-3.0 (cliente) | Electron | Para Pro | No | Sí |
| Claws Mail | GPL-3.0 | Sí (GTK) | No | N/A | Mayormente |

## Por qué el open-source importa aquí

El correo es el vector de restablecimiento de contraseña para todo lo demás. Un cliente cerrado puede, intencionalmente o no, exfiltrar credenciales o indexar en servidor. Un cliente nativo, abierto, local-first (Thunderbird, Betterbird, Skim) reduce la superficie de confianza a app y API que puedes leer.

## FAQ

**¿Mejor cliente open-source para Windows 2026?** Thunderbird para todo, Skim para rápido, Betterbird para Thunderbird-sin-asperezas.

**¿Thunderbird portable en Windows?** Portableapps.com.

**¿El nuevo Outlook es open-source?** No.

**¿Skim tiene ecosistema de extensiones como Thunderbird?** No, intencionalmente.

**¿Auto-alojar back-end de Mailspring?** No, solo el cliente es abierto.

**¿Thunderbird tiene IA?** No integrada. Add-ons. Skim trae IA en el producto en tu clave, [AI](/#ai).
