# Segurança e persistência do RustiQ

## O que é protegido

O `workspace.json` usa o formato versionado `rustiq-encrypted-workspace` (v3). O formato v2 continua legível e é atualizado
na próxima gravação.
Nomes de coleções, pastas e requests, IDs, hierarquia, métodos HTTP e preferências
visuais permanecem legíveis. Não coloque credenciais nos nomes dos itens.

URLs e toda a configuração de cada request são criptografadas: parâmetros,
headers, cookies configurados, autenticação e body, incluindo campos desativados.
A proteção abrange campos arbitrários; não depende de reconhecer nomes como
`Authorization` ou `password`. O histórico persiste somente os bodies e headers
da request preparada e da resposta recebida, dentro do mesmo conteúdo
criptografado. `Cookie`, `Set-Cookie`, `Cookie2` e `Set-Cookie2` são excluídos do
histórico, independentemente de maiúsculas/minúsculas. Os cookies capturados
continuam somente na memória; a configuração de cookies da request mantém a
proteção criptografada já existente.

O histórico é associado à request e contém data UTC, status, duração e tamanho
para identificar cada troca. Não guarda URLs ou cópias completas de auth/params
em cada registro. Headers de autenticação que efetivamente fazem parte da
request preparada são mantidos criptografados. Headers de transporte que o
cliente HTTP adiciona depois da preparação podem não aparecer no snapshot.
Corpos são preservados como texto, como no editor de resposta existente. Se o
servidor escrever dados de cookies dentro do body, esse conteúdo faz parte do
body e não é removido automaticamente.

São registrados apenas envios que receberam uma resposta HTTP completa,
incluindo 4xx/5xx. Falhas de conexão e envios cancelados não criam um par
request/response. Cada snapshot é independente do draft atual: editar a request
durante o envio ou usar prettify no histórico não muda o que foi registrado.

O botão **History** abre a consulta lado a lado dos bodies e headers, com cópia,
prettify JSON e retorno ao texto original. **Clear history** exige confirmação
e remove os registros dessa request na próxima gravação bem-sucedida.
A retenção é de até 100 trocas por request e 8 MiB de payloads por workspace,
removendo os registros mais antigos. Uma troca acima de 2 MiB é ignorada com
aviso, sem truncar o payload ou impedir a exibição da resposta atual.

A criptografia é AES-256-GCM, com nonce aleatório de 96 bits em cada gravação.
Os metadados também são autenticados como dados associados (AAD). Alterações no
conteúdo criptografado ou nos metadados fazem a abertura falhar; o aplicativo não
substitui o arquivo por um workspace vazio.

## Chave no sistema operacional

Cada workspace recebe uma chave aleatória de 256 bits, independente das outras.
Ela é guardada com o serviço `io.rustiq.desktop.workspace-key` e um identificador
UUID no cofre nativo:

- Linux: Secret Service via D-Bus, como GNOME Keyring ou KWallet com suporte a
  Secret Service habilitado. É necessária uma sessão de usuário com o serviço
  instalado e desbloqueado.
- macOS: Keychain Services.
- Windows: Credential Manager.

Nenhuma cópia da chave é gravada junto ao JSON. Não há fallback para chave fixa,
chave derivada do nome da máquina ou armazenamento de segredos em texto puro.
Uma chave nova só é usada depois de confirmar sua leitura no cofre.

Ao abrir o workspace, a chave fica em memória até ele ser bloqueado ou o
aplicativo ser encerrado. O bloqueio do cofre do SO não bloqueia automaticamente
um workspace já aberto. Use o botão **Lock** para fechá-lo e descartar a chave em
memória. Buffers de chave e de serialização são limpos com `zeroize`; isso não é
uma garantia de apagamento de todas as cópias existentes em memória ou swap.

## Somente nesta sessão

Desmarque **Save request configuration encrypted** para excluir a configuração
da request da próxima gravação. A estrutura e o nome do item permanecem; URL e
campos e histórico são descartados ao fechar ou bloquear o workspace. A alteração só é
persistida quando a gravação conclui; um erro é mostrado se isso falhar.

Se o cofre não estiver disponível, é possível tentar novamente ou abrir um
**temporary workspace**. Nesse modo nada é persistido, incluindo o histórico, e o arquivo existente não
é alterado. Reinicie o aplicativo para voltar ao workspace salvo.

## Migração e gravação

Encerre versões antigas do RustiQ antes de migrar: elas não conhecem o novo
formato nem o protocolo de lock. A versão v3 não pode ser aberta por versões
que aceitam somente o formato v2; elas recusam a abertura sem sobrescrever o
arquivo.

O JSON antigo é migrado automaticamente na primeira abertura bem-sucedida. A
migração valida o conteúdo e confirma a chave no SO antes de substituir o
arquivo. Em caso de erro, o original é preservado. Não é criada uma cópia de
backup em texto puro; o temporário `workspace.json.tmp` usado pela versão antiga
é removido depois de uma migração bem-sucedida.

As gravações usam um temporário de nome aleatório no mesmo diretório,
sincronização em disco e substituição atômica. Em Unix, arquivos de snapshot e
lock são criados com permissão `0600`, e novos diretórios com `0700`. Um lock de
arquivo e a comparação do snapshot anterior impedem que duas instâncias
sobrescrevam silenciosamente as alterações uma da outra. Os metadados legíveis
não devem ser editados manualmente, pois fazem parte da autenticação do arquivo.

O caminho respeita `XDG_CONFIG_HOME` quando definido e absoluto. Caso contrário,
usa o diretório de configuração do usuário fornecido pelo SO, com a subpasta
`rustiq`. Uma instalação anterior em `~/.config/rustiq` é mantida quando não
existe arquivo no novo caminho nativo.

## Limites e recuperação

Copiar somente o JSON não permite recuperar as requests sem a chave do SO.
Entretanto, malware executado na conta do usuário pode conseguir consultar o
cofre ou capturar dados de um workspace desbloqueado. O recurso não promete
proteção contra um computador já comprometido.

O bloqueio cancela as tarefas HTTP ainda em andamento no cliente. Não desfaz
requisições que já chegaram ao servidor. Conteúdo copiado explicitamente para
a área de transferência também fica fora do armazenamento criptografado.

**Faça backup do cofre do SO junto com o arquivo.** Copiar somente o JSON para
outra máquina ou perder a chave torna o conteúdo inacessível. O RustiQ não gera
uma chave substituta para um arquivo existente e não possui chave mestra de
recuperação nem exportação de chave nesta versão.

A migração não apaga versões antigas em backups, Git, snapshots de disco ou
áreas recuperáveis do SSD. Se credenciais já foram expostas, criptografar a
versão atual não substitui sua revogação e rotação.

## Validação

`cargo test --workspace` usa cofres simulados e servidores HTTP locais. Os testes
cobrem ausência de texto puro, migração, adulteração, chave incorreta ou ausente,
falhas de gravação, concorrência, modo de sessão e ocultação em Debug.

O teste abaixo usa o cofre nativo e cria e remove apenas uma chave descartável;
é ignorado nos testes comuns para não depender de uma sessão gráfica:

```sh
cargo test -p rustiq-desktop native_keychain_round_trip -- --ignored
```

Os backends macOS e Windows precisam ser verificados nos respectivos sistemas;
a validação nativa realizada neste ambiente cobre Linux.
