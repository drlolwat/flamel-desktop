import {execFileSync} from 'node:child_process';
import {mkdirSync,copyFileSync} from 'node:fs';
import {resolve,join} from 'node:path';
const profile=process.argv[2]??'debug';
if(!['debug','release'].includes(profile))throw new Error('profile must be debug or release');
const tools=['flamel_mcp','git_mcp','docs_mcp','db_mcp','browser_mcp','memory_mcp'];
execFileSync('cargo',['build','--locked','-p','flamel-engine',...(profile==='release'?['--release']:[]),...tools.flatMap(tool=>['--bin',tool])],{stdio:'inherit'});
const metadata=JSON.parse(execFileSync('cargo',['metadata','--no-deps','--format-version','1'],{encoding:'utf8'}));
const destination=resolve('desktop/resources/bin');mkdirSync(destination,{recursive:true});
for(const tool of tools){const name=tool+(process.platform==='win32'?'.exe':'');copyFileSync(join(metadata.target_directory,profile,name),join(destination,name));}
