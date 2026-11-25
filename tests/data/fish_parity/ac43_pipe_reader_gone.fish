seq 1 100000 | string upper | head -1; echo $pipestatus
status get-file help_sections | head -2; echo $status
status get-file nosuchfile; echo $status
status list-files functions/ls | head -3
