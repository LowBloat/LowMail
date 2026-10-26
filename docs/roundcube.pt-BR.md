# Substituindo o Roundcube pelo LowMail

O LowMail usa os mesmos protocolos que o Roundcube: **IMAP** para ler, **SMTP** (submission) para enviar e **ManageSieve** para filtros e resposta automática. Nada muda no servidor de e-mail; basta apontar o LowMail para os mesmos endereços que o `config.inc.php` do Roundcube usa.

## 1. Descubra os servidores que o Roundcube usa

No `config/config.inc.php` do Roundcube:

| Roundcube | LowMail |
| --- | --- |
| `$config['imap_host'] = 'ssl://mail.exemplo.com.br:993';` | `LOWMAIL_IMAP_HOST=mail.exemplo.com.br`, `LOWMAIL_IMAP_SECURITY=tls`, `LOWMAIL_IMAP_PORT=993` |
| `$config['imap_host'] = 'tls://mail.exemplo.com.br:143';` | `LOWMAIL_IMAP_SECURITY=starttls`, `LOWMAIL_IMAP_PORT=143` |
| `$config['smtp_host'] = 'tls://mail.exemplo.com.br:587';` | `LOWMAIL_SMTP_HOST=mail.exemplo.com.br`, `LOWMAIL_SMTP_SECURITY=starttls`, `LOWMAIL_SMTP_PORT=587` |
| `$config['smtp_host'] = 'ssl://mail.exemplo.com.br:465';` | `LOWMAIL_SMTP_SECURITY=tls`, `LOWMAIL_SMTP_PORT=465` |
| `%d` ou `%h` no host | `{domain}`, por exemplo `mail.{domain}` (defina também `LOWMAIL_ALLOWED_DOMAINS`) |
| `$config['username_domain'] = 'exemplo.com.br';` | `LOWMAIL_DOMAIN=exemplo.com.br` |
| plugin `managesieve` (`managesieve_host`, porta 4190) | `LOWMAIL_SIEVE_HOST`, `LOWMAIL_SIEVE_PORT=4190`, `LOWMAIL_SIEVE_SECURITY=starttls` |
| `$config['imap_conn_options']` com `verify_peer => false` | `LOWMAIL_ACCEPT_INVALID_CERTS=true` (só para certificado autoassinado) |

## 2. Suba o LowMail

Edite o [`compose.yaml`](../compose.yaml):

```yaml
environment:
  LOWMAIL_IMAP_HOST: "mail.exemplo.com.br"
  LOWMAIL_SMTP_HOST: "mail.exemplo.com.br"
  LOWMAIL_DOMAIN: "exemplo.com.br"
  LOWMAIL_ALLOWED_DOMAINS: "exemplo.com.br"
  LOWMAIL_SECURE_COOKIES: "true"
  LOWMAIL_TRUST_PROXY: "true"
  LOWMAIL_PRODUCT_NAME: "Webmail Exemplo"
```

```sh
docker compose up -d --build
docker compose logs -f lowmail
```

Antes de trocar o endereço público, rode a verificação com uma conta de teste. Ela pede a senha sem mostrá-la, testa pastas, leitura, busca, rascunho (apagado depois), pasta temporária (apagada depois), filtros (só leitura) e login no SMTP; `--send-to` envia uma mensagem de teste:

```sh
docker compose run --rm -it lowmail check teste@exemplo.com.br --send-to teste@exemplo.com.br
```

Depois teste em `http://<servidor>:8480` pelo navegador.

## 3. Coloque atrás do HTTPS

O LowMail escuta HTTP na porta 8480; o proxy cuida do certificado. Os eventos de e-mail novo usam Server-Sent Events, então desligue o buffer do proxy para `/api/events`.

**Nginx**

```nginx
server {
    listen 443 ssl http2;
    server_name webmail.exemplo.com.br;
    # ssl_certificate ...;

    client_max_body_size 60m;   # anexos de até 50 MB

    location / {
        proxy_pass http://127.0.0.1:8480;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location /api/events {
        proxy_pass http://127.0.0.1:8480;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_buffering off;
        proxy_read_timeout 1h;
    }
}
```

**Caddy**

```
webmail.exemplo.com.br {
    request_body {
        max_size 60MB
    }
    reverse_proxy 127.0.0.1:8480 {
        flush_interval -1
    }
}
```

Para manter o mesmo endereço do Roundcube em um subcaminho (por exemplo `https://exemplo.com.br/webmail/`), remova o prefixo no proxy: o LowMail usa caminhos relativos e funciona em qualquer pasta.

```nginx
location /webmail/ {
    proxy_pass http://127.0.0.1:8480/;
    # mesmos cabeçalhos do exemplo acima
}
```

## 4. O que muda para os usuários

- **Senha e caixa postal**: as mesmas. Pastas, mensagens e sinalizações estão no servidor e aparecem do mesmo jeito.
- **Filtros**: o LowMail grava os próprios filtros em um script Sieve chamado `lowmail` e o ativa ao salvar. Se o usuário tinha filtros do Roundcube (script `roundcube`), a tela de filtros avisa e mostra o script atual; ele continua guardado no servidor e pode ser reativado.
- **Resposta automática**: fica no mesmo script Sieve, com datas de início e fim.
- **Contatos**: o catálogo do Roundcube fica no banco dele. Exporte em *Contatos → Exportar vCard* no Roundcube e importe em *Contatos → Importar* no LowMail (vCard ou CSV).
- **Identidades e assinaturas**: crie em *Configurações → Identidades e assinaturas*. A primeira é criada com o endereço de login.
- **Preferências**: idioma, tema, painel de leitura, imagens externas e outras opções ficam por usuário no `lowmail.db`.

## 5. Backup

O volume `/data` guarda só o `lowmail.db` (preferências, identidades e contatos). Senhas e mensagens não ficam no LowMail. Para copiar:

```sh
docker compose exec lowmail sh -c 'cat /data/lowmail.db' > lowmail-backup.db
```

## Problemas comuns

| Sintoma | Causa provável |
| --- | --- |
| "O certificado do servidor não é confiável" | Certificado autoassinado ou nome diferente do host. Use o nome do certificado em `LOWMAIL_IMAP_HOST` ou `LOWMAIL_ACCEPT_INVALID_CERTS=true`. |
| Login falha mas a senha está certa | O servidor espera só a parte antes do `@`: `LOWMAIL_LOGIN_LOCAL_PART=true`. |
| Envio falha com "outgoing server" | Porta/segurança do SMTP: 587 + `starttls` ou 465 + `tls`. |
| "Filtros indisponíveis" | ManageSieve desligado no servidor (Dovecot Pigeonhole) ou porta 4190 bloqueada. Use `LOWMAIL_SIEVE=false` para esconder a tela. |
| Mensagens enviadas aparecem duas vezes em Enviados | O servidor já guarda a cópia (alguns provedores fazem isso). Desligue *Configurações → Escrita → Guardar cópia em Enviados*. |
| E-mail novo demora a aparecer | O proxy está segurando `/api/events`; veja `proxy_buffering off` acima. |

Use `LOWMAIL_LOG=debug` para ver a conversa com os servidores.
