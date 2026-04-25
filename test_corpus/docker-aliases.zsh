#{{{                    MARK:Docker
#**************************************************************
alias dk='docker'
alias dka='docker attach'
alias dkb='docker build'
alias dkd='docker diff'
alias dkdf='docker system df'
alias dke='docker exec'
alias dkE='docker exec -e COLUMNS=$(tput cols) -e LINES=$(tput lines) -i -t'
alias dkh='docker history'
alias dkis='docker images'
alias dkin='docker inspect'
alias dkim='docker import'
alias dkk='docker kill'
alias dkkh='docker kill -s HUP'
alias dkl='docker logs'
alias dkL='docker logs -f'
alias dkli='docker login'
alias dklo='docker logout'
alias dkls='docker ps'
alias dkp='docker pause'
alias dkP='docker unpause'
alias dkpl='docker pull'
alias dkph='docker push'
alias dkps='docker ps'
alias dkpsa='docker ps -a'
alias dkpsv='docker ps --format="ID\t{{.ID}}\nNAME\t{{.Names}}\nIMAGE\t{{.Image}}\nPORTS\t{{.Ports}}\nCOMMAND\t{{.Command}}\nCREATED\t{{.CreatedAt}}\nSTATUS\t{{.Status}}\n"'
alias dkr='docker run'
alias dkR='docker run -e COLUMNS=$(tput cols) -e LINES=$(tput lines) -i -t --rm'
alias dkRe='docker run -e COLUMNS=$(tput cols) -e LINES=$(tput lines) -i -t --rm --entrypoint /bin/bash'
alias dkRM='docker system prune'
alias dkrm='docker rm'
alias dkrmi='docker rmi'
alias dkrn='docker rename'
alias dks='docker start'
alias dkS='docker restart'
alias dkss='docker stats'
alias dksv='docker save'
alias dkt='docker tag'
alias dktop='docker top'
alias dkup='docker update'
alias dkv='docker volume'
alias dkvs='docker version'
alias dkw='docker wait'
alias dkx='docker stop'
#}}}***********************************************************

#{{{                    MARK:container
#**************************************************************
alias dkc='docker container'
alias dkca='docker container attach'
alias dkccp='docker container cp'
alias dkcd='docker container diff'
alias dkce='docker container exec -e COLUMNS=$(tput cols) -e LINES=$(tput lines) -i -t'
alias dkcin='docker container inspect'
alias dkck='docker container kill'
alias dkcl='docker container logs'
alias dkcL='docker container logs -f'
alias dkcls='docker container ls'
alias dkcp='docker container pause'
alias dkcpr='docker container prune'
alias dkcrn='docker container rename'
alias dkcS='docker container restart'
alias dkcrm='docker container rm'
alias dkcr='docker container run -e COLUMNS=$(tput cols) -e LINES=$(tput lines) -i -t '
alias dkcR='docker container run -e COLUMNS=$(tput cols) -e LINES=$(tput lines) -i -t --rm'
alias dkcre='docker container run -e COLUMNS=$(tput cols) -e LINES=$(tput lines) -i -t --rm --entrypoint /bin/bash'
alias dkcrd='docker container run -e COLUMNS=$(tput cols) -e LINES=$(tput lines) -d'
alias dkcs='docker container start'
alias dkcss='docker container stats'
alias dkcx='docker container stop'
alias dkctop='docker container top'
alias dkcP='docker container unpause'
alias dkcup='docker container update'
alias dkcw='docker container wait'
#}}}***********************************************************

#{{{                    MARK:image
#**************************************************************
alias dki='docker image'
alias dkib='docker image build'
alias dkih='docker image history'
alias dkiim='docker image import'
alias dkiin='docker image inspect'
alias dkils='docker image ls'
alias dkipr='docker image prune'
alias dkipl='docker image pull'
alias dkiph='docker image push'
alias dkirm='docker image rm'
alias dkisv='docker image save'
alias dkit='docker image tag'
#}}}***********************************************************

#{{{                    MARK:volume
#**************************************************************
alias dkv='docker volume'
alias dkvin='docker volume inspect'
alias dkvls='docker volume ls'
alias dkvpr='docker volume prune'
alias dkvrm='docker volume rm'
#}}}***********************************************************

#{{{                    MARK:network
#**************************************************************
alias dkn='docker network'
alias dkns='docker network connect'
alias dknx='docker network disconnect'
alias dknin='docker network inspect'
alias dknls='docker network ls'
alias dknpr='docker network prune'
alias dknrm='docker network rm'
#}}}***********************************************************

#{{{                    MARK:system
#**************************************************************
alias dky='docker system'
alias dkydf='docker system df'
alias dkypr='docker system prune'
#}}}***********************************************************

#{{{                    MARK:stack
#**************************************************************
alias dkk='docker stack'
alias dkkls='docker stack ls'
alias dkkps='docker stack ps'
alias dkKrm='docker stack rm'
#}}}***********************************************************

#{{{                    MARK:misc
#**************************************************************
alias dkw='docker swarm'

# Clean up exited containers (docker < 1.13)
alias dkrmc='docker rm $(docker ps -qaf status=exited)'

# Clean up dangling images (docker < 1.13)
alias dkrmi='docker rmi $(docker images -qf dangling=true)'

# Pull all tagged images
alias dkpli='docker images --format "{{ .Repository }}" | grep -v "^<none>$" | xargs -L1 docker pull'

# Clean up dangling volumes (docker < 1.13)
alias dkrmv='docker volume rm $(docker volume ls -qf dangling=true)'
#}}}***********************************************************

#{{{                    MARK:machine
#**************************************************************
alias dkm='docker-machine'
alias dkma='docker-machine active'
alias dkmcp='docker-machine scp'
alias dkmin='docker-machine inspect'
alias dkmip='docker-machine ip'
alias dkmk='docker-machine kill'
alias dkmls='docker-machine ls'
alias dkmpr='docker-machine provision'
alias dkmps='docker-machine ps'
alias dkmrg='docker-machine regenerate-certs'
alias dkmrm='docker-machine rm'
alias dkms='docker-machine start'
alias dkmsh='docker-machine ssh'
alias dkmst='docker-machine status'
alias dkmS='docker-machine restart'
alias dkmu='docker-machine url'
alias dkmup='docker-machine upgrade'
alias dkmv='docker-machine version'
alias dkmx='docker-machine stop'
#}}}***********************************************************

#{{{                    MARK:compose
#**************************************************************
alias dco='docker-compose'
alias dcb='docker-compose build'
alias dcB='docker-compose build --no-cache'
alias dcd='docker-compose down'
alias dce='docker-compose exec -e COLUMNS=$(tput cols) -e LINES=$(tput lines)'
alias dck='docker-compose kill'
alias dclo='docker-compose logs'
alias dcL='docker-compose logs -f'
alias dcls='docker-compose ps'
alias dcp='docker-compose pause'
alias dcP='docker-compose unpause'
alias dcpl='docker-compose pull'
alias dcph='docker-compose push'
alias dcps='docker-compose ps'
alias dcr='docker-compose run -e COLUMNS=$(tput cols) -e LINES=$(tput lines)'
alias dcR='docker-compose run -e COLUMNS=$(tput cols) -e LINES=$(tput lines) --rm'
alias dcrm='docker-compose rm'
alias dcs='docker-compose start'
alias dcsc='docker-compose scale'
alias dcS='docker-compose restart'
alias dcU='docker-compose up'
alias dcu='docker-compose up -d'
alias dcv='docker-compose version'
alias dcx='docker-compose stop'
#}}}***********************************************************
