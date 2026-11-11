source @FN@/prompt_pwd.fish
cd /usr/local/bin
prompt_pwd
set -g fish_prompt_pwd_dir_length 3
prompt_pwd
set -g fish_prompt_pwd_dir_length 0
prompt_pwd
prompt_pwd /usr/share/doc /etc
