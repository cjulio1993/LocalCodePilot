# Assinatura do binário para Windows

O Windows verifica a assinatura Authenticode do arquivo `LocalCodePilot.exe`. Assinar apenas o ZIP não identifica o editor do aplicativo.

## Certificado necessário

Uma distribuição pública precisa de um certificado de assinatura de código emitido por uma autoridade confiável. Certificados autoassinados servem para testes internos, mas não removem os avisos apresentados a usuários públicos.

Há duas opções adequadas:

1. **Microsoft Artifact Signing:** a chave fica protegida no serviço e o GitHub Actions assina o executável sem armazenar um arquivo `.pfx` no repositório. Exige uma conta, validação de identidade e um perfil de certificado. A disponibilidade de certificados Public Trust depende do país da identidade validada.
2. **Certificado OV ou EV de uma autoridade certificadora:** o SignTool usa a chave fornecida em hardware, no repositório de certificados do Windows ou por uma integração segura do provedor.

Nunca versione um `.pfx`, token, senha ou chave privada.

## Artifact Signing no pipeline

O workflow `.github/workflows/release-windows.yml` já possui uma etapa opcional com `Azure/artifact-signing-action@v1`. Configure estes secrets no repositório GitHub:

- `ARTIFACT_SIGNING_ENDPOINT`
- `ARTIFACT_SIGNING_ACCOUNT`
- `ARTIFACT_SIGNING_PROFILE`
- `AZURE_TENANT_ID`
- `AZURE_CLIENT_ID`
- `AZURE_CLIENT_SECRET`

A identidade do aplicativo Azure precisa da função **Artifact Signing Certificate Profile Signer** no perfil usado. Quando todos os secrets estiverem disponíveis, o workflow assina o `.exe`, valida a assinatura e somente depois monta o ZIP. Sem eles, a alpha é empacotada sem assinatura e o resumo do workflow registra essa condição.

Para reduzir o uso de segredos de longa duração, substitua posteriormente o client secret por autenticação OIDC entre GitHub e Azure.

## Assinatura local com SignTool

Com o certificado instalado no repositório do Windows:

```powershell
signtool sign /fd SHA256 /a /tr http://timestamp.digicert.com /td SHA256 .\LocalCodePilot.exe
```

Com um PFX mantido fora do repositório:

```powershell
signtool sign /fd SHA256 /f C:\segredos\codesign.pfx /tr http://timestamp.digicert.com /td SHA256 .\LocalCodePilot.exe
```

O SignTool solicitará ou receberá a senha por um mecanismo seguro do ambiente. Evite colocar a senha no histórico do terminal.

Valide o resultado antes de empacotar:

```powershell
signtool verify /pa /v .\LocalCodePilot.exe
Get-AuthenticodeSignature .\LocalCodePilot.exe | Format-List Status,StatusMessage,SignerCertificate
```

O carimbo de tempo RFC 3161 mantém a assinatura verificável depois que o certificado expira. Uma assinatura válida exibe o editor verificado, mas o Microsoft Defender SmartScreen ainda pode alertar sobre uma versão nova até que ela obtenha reputação suficiente.

Referências oficiais:

- [SmartScreen reputation for app developers](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation)
- [Artifact Signing quickstart](https://learn.microsoft.com/azure/artifact-signing/quickstart)
- [Artifact Signing integrations](https://learn.microsoft.com/azure/artifact-signing/how-to-signing-integrations)
- [SignTool signing](https://learn.microsoft.com/windows/win32/seccrypto/using-signtool-to-sign-a-file)
