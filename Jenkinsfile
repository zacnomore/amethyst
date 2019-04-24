pipeline {
    agent none
    stages {
        stage('Pull new images') {
            agent {
                label 'docker'
            }
            steps {
                sh 'docker pull amethystrs/builder-linux:stable'
                sh 'docker pull amethystrs/builder-linux:nightly'
            }
        }
        stage('Cargo Fmt') {
            environment {
                RUSTFLAGS = '-D warnings'
            }
            agent {
                docker {
                    image 'amethystrs/builder-linux:stable'
                    label 'docker'
                } 
            }
            steps {
                echo 'Checking formatting...'
                sh 'cargo fmt -- --check'
            }
        }
        stage('Cargo Check') {
            parallel {
                stage('stable') {
                    environment {
                        RUSTFLAGS = '-D warnings'
                    }
                    agent {
                        docker {
                            image 'amethystrs/builder-linux:stable'
                            label 'docker'
                        } 
                    }
                    steps {
                        echo 'Running Cargo check...'
                        sh 'cargo check --all --all-targets --features sdl_controller,json,saveload'
                    }
                }
                stage('nightly') {
                    environment {
                        RUSTFLAGS = '-D warnings'
                    }
                    agent {
                        docker {
                            image 'amethystrs/builder-linux:nightly'
                            label 'docker'
                        } 
                    }
                    steps {
                        echo 'Running Cargo check...'
                        sh 'cargo check --all --all-targets --features nightly'
                    }
                }
            }
        }
        stage('Run Tests') {
            parallel {
                stage('Test on Windows') {
                    environment {
                        CARGO_HOME = 'C:\\Users\\root\\.cargo'
                        RUSTUP_HOME = 'C:\\Users\\root\\.rustup'
                    }
                    agent {
                        label 'windows' 
                    }
                    steps {
                        echo 'Beginning tests...'
                        bat 'C:\\Users\\root\\.cargo\\bin\\cargo test --all'
                        echo 'Tests done!'
                    }
                }
                stage('Test on Linux') {
                    agent {
                        docker {
                            image 'amethystrs/builder-linux:stable'
                            label 'docker'
                        }
                    }
                    steps {
                        echo 'Beginning tests...'
                        sh 'cargo test --all'
                        echo 'Tests done!'
                    }
                }
                // macOS is commented out due to needing to upgrade the OS, but MacStadium did not do the original install with APFS so we cannot upgrade easily
                // stage('Test on macOS') {
                //     environment {
                //         CARGO_HOME = '/Users/jenkins/.cargo'
                //         RUSTUP_HOME = '/Users/jenkins/.rustup'
                //     }
                //     agent {
                //         label 'mac'
                //     }
                //     steps {
                //         echo 'Beginning tests...'
                //         sh '/Users/jenkins/.cargo/bin/cargo test --all'
                //         echo 'Tests done!'
                //     }
                // }
            }
        }
        stage('Build and Upload Docs') {
            // Only update the docs site when a commit makes it to master.
            when { branch 'master' }
            stages {
                stage('Build Docs') {
                    parallel {
                        stage('Build API Docs') {
                            agent {
                                docker {
                                    image 'amethystrs/builder-linux:stable'
                                    label 'docker'
                                } 
                            }
                            // Generate master and all tagged releases of the API docs
                            steps {
                                echo 'Generating API docs for the current commit...'
                                sh '''#!/bin/bash

                                    # fetch every tag and then add the master branch to a variable
                                    tags=$(git tag)
                                    branches="$tags master"

                                    # loop over everything for which we want to create API documentation
                                    for branch in $branches
                                    do
                                        [ "$branch" = "master" ] && git checkout master || git checkout "tags/$branch"
                                        echo "Building API docs for $branch"
                                        cargo doc --all --no-deps --quiet --target-dir "doc/$branch/"

                                        # files are generated into a "doc" folder. Move those files to our API folder
                                        mkdir doc/$branch/api
                                        cp -rp doc/$branch/doc/* doc/$branch/api/

                                        # delete unneeded files
                                        rm -rf doc/$branch/debug doc/$branch/doc doc/$branch/.rustc_info.json
                                    done
                                    '''
                                echo 'API docs generation done!'
                            }
                        }
                        stage('Build the Book') {
                            agent {
                                docker {
                                    image 'amethystrs/builder-linux:stable'
                                    label 'docker'
                                } 
                            }
                            // Generate master and all tagged releases of the Book
                            steps {
                                echo 'Generating the Amethyst Book for the current commit...'
                                sh '''#!/bin/bash

                                    # fetch every tag and then add the master branch to a variable
                                    tags=$(git tag)
                                    branches="$tags master"

                                    # loop over everything for which we want to create API documentation
                                    for branch in $branches
                                    do
                                        [ "$branch" = "master" ] && git checkout master || git checkout "tags/$branch"
                                        echo "Building Book for $branch"
                                        mdbook build book --dest-dir ../doc/$branch/book/
                                    done
                                    '''
                                echo 'Amethyst Book generation done!'
                            }
                        }
                    }
                }
                stage('Upload Docs') {
                    agent {
                        docker {
                            image 'amethystrs/builder-linux:stable'
                            label 'docker'
                        } 
                    }
                    steps {
                        // API Docs root path: doc/{branch}/api
                        // Book root path:     doc/{branch}/book
                        sh '''#!/bin/bash
                            # We also want the "latest" tag in a copy of the folder.
                            cp -r doc/$(git describe --tags `git rev-list --tags --max-count=1`) doc/latest/
                        '''
                    }
                }
            }
        }
    }
    post {
        always {
            archiveArtifacts artifacts: 'doc/**/*', fingerprint: true
        }
    }
}
