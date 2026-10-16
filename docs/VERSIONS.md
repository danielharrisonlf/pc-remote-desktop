# Versões verificadas em 16/10/2026

Os lockfiles são a referência exata de todas as dependências transitivas. As versões abaixo foram usadas nas builds locais que acompanham esta entrega; não são versões beta.

| Componente | Versão |
|---|---|
| Desktop / Android / protocolo | 0.3.0 / 0.3.0-kotlin / 1 + extensão opcional input.v1 |
| Node / npm | 24.21.0 / 11.19.0 |
| Rust / MSVC / Windows SDK de build | 1.97.1 / 19.44 / 10.0.26100.0 |
| Tauri Rust / CLI / API JS | 2.12.3 / 2.12.2 / 2.12.3 |
| Tauri Single Instance | 2.5.3 |
| windows crate | 0.62.2 |
| Desktop React / TypeScript / Vite | 19.3.0 / 5.9.3 / 8.3.4 |
| Vitest | 5.0.4 |
| Android: Kotlin integrado ao AGP / Android Gradle Plugin / Gradle | 2.2.10 / 9.2.1 / 9.4.1 |
| Android: JDK | Temurin 17.0.20.1 |
| Android: compile / target / mínimo | 37 / 36 / 24 |
| Android: pacote SDK / Build Tools | android-37.0 / 37.0.0 |
| Android: OkHttp / Google Play Services Code Scanner | 4.12.0 / 16.1.0 |

O app Android é Kotlin com Views nativas, sem React Native, Hermes, Compose ou NDK. A build de release não depende de servidor de desenvolvimento.

Target Android 36: o acesso LAN usa INTERNET e NSD. Ao elevar target para 37, implementar ACCESS_LOCAL_NETWORK com solicitação contextual ou o seletor autorizado do sistema, conforme [documentação Android](https://developer.android.com/privacy-and-security/local-network-permission). Não usar serviço em primeiro plano como alternativa à restrição de clipboard.

## Compatibilidade entre versões

| Android | Desktop | Compatibilidade |
|---|---|---|
| 0.3.0-kotlin | 0.3.0 | Protocolo 1; input.v1 após consentimento nos dois aplicativos; o PC mostra a mesma faixa de cores do pareamento |
| 0.3.0-kotlin | 0.2.x | Mesmo contrato; o PC mostra a identidade só em hexadecimal |
| 0.2.0–0.2.4-kotlin | 0.2.x e 0.3.0 | Compatibilidade anterior preservada; input.v1 após consentimento |
| 0.2.x e 0.3.0 | 0.1.x | Volume, mídia e clipboard; sem mouse/teclado |
| Qualquer implementação | Sem protocolo 1 | Incompatível; rejeitado |

A matriz descreve contrato; capacidades do dispositivo/player definem os controles reais. Não equivale a certificação de todos os players/hardwares.
