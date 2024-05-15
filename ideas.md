Current build process:

0. build is started from the <systype> folder??? or in the build/<systype> folder?? can't remember how this works but it can make buillding using idf.py possible even for different systypes???
1. Fetch RaftCore and place in a sub-folder of build/<systype>
- GIT_TAG needs to be specifiable
- GIT_REPOSITORY needs to be specifiable
2. Either use the <systype> given as a param or determine a <systype> to use - maybe from platformio.ini if it is being used? or just the first one found if not? (in RaftProject.cmake)
3. Set PROJECT_BASENAME which will be <systype>
4. Make sure the folder <build>/<systype> exists and use this as the build folder
5. Create a BUILD_ARTIFACTS folder inside <build>/<systype>
6. Write a "cursystype.text" file containing <systype> into the BUILD_ARTIFACTS folder - not necessary as BUILD_ARTIFACTS now inside <build>/<systype> - it wasn't before solely (I think) to allow a fixed location to be specified in sdkconfig.defaults for partitions.csv file???
7. Not currenyly done but sdkconfig.defaults could be altered to account for different partitions.csv paths? But altering it would probably trigger a full build? Maybe that's ok though since it would only get altered if the <systype> actually changed? Or make a copy of sdkconfig.default into the build folder and modify that version and then set SDKCONFIG_DEFAULTS below to use that? 
8. Specify the SDKCONFIG_DEFAULTS and SDKCONFIG file names
9. Ensure changing sdkconfig.defaults results in a rebuild of sdkconfig - MAYBE could simply delete sdkconfig if sdkconfig.defaults has changed since the last build?
10. Process the systypes header file using the GenerateSysTypes script
11. Include the specific features defined in systypes/<systype>/features.cmake
12. Set IDF_TARGET
13. Run ESP IDF build script
14. Set FW_IMAGE_NAME
15. Add OPTIONAL_COMPONENT folders
16. Set the location of the partitions.csv file - currently this involves copying it to the BUILD_RAFT_ARTIFACTS folder but that is different for each build although this is ok because the sdkconfig.defaults can different for each build too
17. Fetch Raft component libraries that are specified in the build - need to specify GIT_TAG, GIT_REPOSITORY
18. Build WebUI (if there is one) using GenWebUI.py script
19. Build FS image with webui files added to the base ones specified
20. Update compile_commands.json if required - there is a different one for each <systype>
21. Run RaftGenFSImage.py which generates the actual binary file to be programmed





110. Add all libraries to the build RaftCore, any other Raft libs, any littlefs and mdns libs, etc